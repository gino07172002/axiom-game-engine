//! File-backed audio clips: WAV (PCM 8/16/24/32-bit integer or 32-bit float) and Ogg Vorbis
//! decoded once to the mixer's format (mono 16-bit at [`SAMPLE_RATE`]).
//!
//! Decoding streams sample by sample through a downmix (channel average) and a resampler, so
//! the only buffer that grows is the output, whose size is bounded before it is allocated
//! (WAV: from the header; Vorbis: checked per packet). Budgets are explicit errors, never a
//! silent truncation: at most [`MAX_FILE_BYTES`] encoded and [`MAX_FILE_SECONDS`] decoded.
use anyhow::{bail, ensure, Context, Result};
use axiom_core::audio::{MAX_FILE_BYTES, MAX_FILE_SECONDS, SAMPLE_RATE};

/// Source sample rates accepted (both containers).
const MIN_RATE: u32 = 8_000;
const MAX_RATE: u32 = 192_000;

/// Mono 16-bit PCM at [`SAMPLE_RATE`] plus what the file held.
#[derive(Debug)]
pub struct Decoded {
    pub pcm: Vec<i16>,
    pub format: &'static str,
    pub channels: u16,
    pub rate: u32,
    /// Duration of the source (frames / rate).
    pub seconds: f64,
}

/// Decodes `bytes` (format by its magic bytes: `RIFF....WAVE` or `OggS`), scaling by `gain`.
pub fn decode(bytes: &[u8], gain: f32) -> Result<Decoded> {
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "audio file is {} bytes (budget {MAX_FILE_BYTES})",
        bytes.len()
    );
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        decode_wav(bytes, gain)
    } else if bytes.starts_with(b"OggS") {
        // The Vorbis decoder is third-party code fed untrusted bytes: a panic in it becomes
        // an error for this file instead of taking the runtime build down.
        std::panic::catch_unwind(|| decode_ogg(bytes, gain))
            .unwrap_or_else(|_| bail!("Ogg Vorbis decoder failed on this file"))
    } else {
        bail!("not a WAV (RIFF/WAVE) or Ogg Vorbis file")
    }
}

fn check_format(channels: u16, rate: u32) -> Result<()> {
    ensure!(
        (1..=2).contains(&channels),
        "{channels} channels: only mono and stereo files are supported"
    );
    ensure!(
        (MIN_RATE..=MAX_RATE).contains(&rate),
        "sample rate {rate} Hz is outside {MIN_RATE}..{MAX_RATE}"
    );
    Ok(())
}

fn too_long(frames: u64, rate: u32) -> anyhow::Error {
    anyhow::anyhow!(
        "audio file is {:.1} s long (budget {MAX_FILE_SECONDS} s)",
        frames as f64 / f64::from(rate)
    )
}

/// Output samples a source of `frames` frames at `rate` becomes (the resampler's count).
fn output_len(frames: u64, rate: u32) -> usize {
    if frames == 0 {
        return 0;
    }
    ((frames - 1) * u64::from(SAMPLE_RATE) / u64::from(rate) + 1) as usize
}

/// Streaming mono resampler to [`SAMPLE_RATE`]: a moving average over `ratio` input samples
/// (anti-aliasing when downsampling) followed by linear interpolation.
struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Input position of the next output sample.
    next: f64,
    /// Input samples seen.
    count: u64,
    previous: f32,
    window: [f32; 32],
    width: usize,
    sum: f32,
    gain: f32,
    out: Vec<i16>,
}

impl Resampler {
    fn new(rate: u32, gain: f32, capacity: usize) -> Self {
        let step = f64::from(rate) / f64::from(SAMPLE_RATE);
        Self {
            step,
            next: 0.0,
            count: 0,
            previous: 0.0,
            window: [0.0; 32],
            width: (step.round() as usize).clamp(1, 32),
            sum: 0.0,
            gain,
            out: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, x: f32) {
        let k = (self.count % self.width as u64) as usize;
        self.sum += x - self.window[k];
        self.window[k] = x;
        let filled = (self.count + 1).min(self.width as u64) as f32;
        let current = self.sum / filled;
        let at = self.count as f64;
        while self.next <= at {
            let frac = (self.next - (at - 1.0)).clamp(0.0, 1.0) as f32;
            let y = if self.count == 0 {
                current
            } else {
                self.previous + (current - self.previous) * frac
            };
            self.out
                .push((y * self.gain * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
            self.next = self.out.len() as f64 * self.step;
        }
        self.previous = current;
        self.count += 1;
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[derive(Clone, Copy, PartialEq)]
enum Sample {
    U8,
    I16,
    I24,
    I32,
    F32,
}

fn decode_wav(bytes: &[u8], gain: f32) -> Result<Decoded> {
    let mut at = 12;
    let mut format: Option<(Sample, u16, u32, usize)> = None;
    let mut data: Option<&[u8]> = None;
    while at + 8 <= bytes.len() && data.is_none() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4) as usize;
        let body = at + 8;
        ensure!(
            size <= bytes.len() - body,
            "WAV chunk {} is truncated ({size} bytes declared, {} present)",
            String::from_utf8_lossy(id),
            bytes.len() - body
        );
        let chunk = &bytes[body..body + size];
        match id {
            b"fmt " => {
                ensure!(chunk.len() >= 16, "WAV fmt chunk is too short");
                let mut tag = u16_at(chunk, 0);
                let channels = u16_at(chunk, 2);
                let rate = u32_at(chunk, 4);
                let align = u16_at(chunk, 12) as usize;
                let bits = u16_at(chunk, 14);
                if tag == 0xFFFE {
                    // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID starts with the real tag.
                    ensure!(chunk.len() >= 26, "WAV extensible fmt chunk is too short");
                    tag = u16_at(chunk, 24);
                }
                let sample = match (tag, bits) {
                    (1, 8) => Sample::U8,
                    (1, 16) => Sample::I16,
                    (1, 24) => Sample::I24,
                    (1, 32) => Sample::I32,
                    (3, 32) => Sample::F32,
                    (1 | 3, _) => bail!("WAV with {bits}-bit samples is not supported (8, 16, 24, 32-bit PCM or 32-bit float)"),
                    _ => bail!("WAV format tag {tag} is not supported (PCM or IEEE float only)"),
                };
                check_format(channels, rate)?;
                ensure!(
                    align == usize::from(channels) * usize::from(bits / 8),
                    "WAV block align {align} does not match {channels} x {bits}-bit samples"
                );
                format = Some((sample, channels, rate, align));
            }
            b"data" => data = Some(chunk),
            _ => {}
        }
        // Chunks are padded to an even size.
        at = body + size + (size & 1);
    }
    let (sample, channels, rate, align) = format.context("WAV has no fmt chunk")?;
    let data = data.context("WAV has no data chunk")?;
    let frames = (data.len() / align) as u64;
    if frames as f64 / f64::from(rate) > f64::from(MAX_FILE_SECONDS) {
        return Err(too_long(frames, rate));
    }
    let width = align / usize::from(channels);
    let mut resampler = Resampler::new(rate, gain, output_len(frames, rate));
    let scale = 1.0 / f32::from(channels);
    for frame in data.chunks_exact(align) {
        let mut mono = 0.0;
        for s in frame.chunks_exact(width) {
            mono += match sample {
                Sample::U8 => (f32::from(s[0]) - 128.0) / 128.0,
                Sample::I16 => f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0,
                Sample::I24 => {
                    (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0
                }
                Sample::I32 => {
                    i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0
                }
                Sample::F32 => {
                    let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    if v.is_finite() {
                        v.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    }
                }
            };
        }
        resampler.push(mono * scale);
    }
    ensure!(frames > 0, "WAV data chunk holds no samples");
    Ok(Decoded {
        pcm: resampler.out,
        format: "wav",
        channels,
        rate,
        seconds: frames as f64 / f64::from(rate),
    })
}

fn decode_ogg(bytes: &[u8], gain: f32) -> Result<Decoded> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes))
        .map_err(|e| anyhow::anyhow!("Ogg Vorbis header: {e}"))?;
    let channels = u16::from(reader.ident_hdr.audio_channels);
    let rate = reader.ident_hdr.audio_sample_rate;
    check_format(channels, rate)?;
    let limit = (f64::from(MAX_FILE_SECONDS) * f64::from(rate)) as u64;
    // Vorbis keeps no total length in its headers: start with a guess the size of the file
    // allows (~1 s per 16 KiB at low bitrates), bounded by the budget.
    let guess = (bytes.len() as u64 / 16_384 + 1) * u64::from(rate);
    let mut resampler = Resampler::new(rate, gain, output_len(guess.min(limit), rate));
    let scale = 1.0 / f32::from(channels) / 32768.0;
    let mut frames = 0u64;
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|e| anyhow::anyhow!("Ogg Vorbis data after {frames} frames: {e}"))?
    {
        frames += (packet.len() / usize::from(channels)) as u64;
        if frames > limit {
            return Err(too_long(frames, rate));
        }
        for frame in packet.chunks_exact(usize::from(channels)) {
            let sum: i32 = frame.iter().map(|&s| i32::from(s)).sum();
            resampler.push(sum as f32 * scale);
        }
    }
    ensure!(frames > 0, "Ogg Vorbis stream holds no samples");
    Ok(Decoded {
        pcm: resampler.out,
        format: "ogg",
        channels,
        rate,
        seconds: frames as f64 / f64::from(rate),
    })
}

/// A WAV file of `samples` (interleaved, -1..1) in `bits`-bit PCM (8 / 16 / 24 / 32) or 32-bit
/// float (`float`). Tests and examples generate their sounds with it; no audio file is
/// third-party.
pub fn encode_wav(samples: &[f32], channels: u16, rate: u32, bits: u16, float: bool) -> Vec<u8> {
    let width = usize::from(bits / 8);
    let mut data = Vec::with_capacity(samples.len() * width);
    for &s in samples {
        let s = s.clamp(-1.0, 1.0);
        match (float, bits) {
            (true, _) => data.extend_from_slice(&s.to_le_bytes()),
            (false, 8) => data.push(((s * 127.0).round() + 128.0) as u8),
            (false, 16) => data.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes()),
            (false, 24) => {
                data.extend_from_slice(&((s * 8_388_607.0).round() as i32).to_le_bytes()[..3])
            }
            (false, _) => data.extend_from_slice(
                &((f64::from(s) * 2_147_483_647.0).round() as i32).to_le_bytes(),
            ),
        }
    }
    let align = usize::from(channels) * width;
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&(if float { 3u16 } else { 1 }).to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * align as u32).to_le_bytes());
    out.extend_from_slice(&(align as u16).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f32, rate: u32, seconds: f32, channels: u16) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .flat_map(|i| {
                let s = 0.5 * (std::f32::consts::TAU * frequency * i as f32 / rate as f32).sin();
                std::iter::repeat_n(s, usize::from(channels))
            })
            .collect()
    }

    /// Upward zero crossings per second of the decoded PCM.
    fn frequency(pcm: &[i16]) -> f32 {
        let ups = pcm.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        ups as f32 * SAMPLE_RATE as f32 / pcm.len() as f32
    }

    #[test]
    fn every_wav_sample_format_decodes_to_the_same_tone() {
        for (bits, float) in [
            (8, false),
            (16, false),
            (24, false),
            (32, false),
            (32, true),
        ] {
            for channels in [1, 2] {
                let wav = encode_wav(
                    &sine(440.0, 22_050, 0.5, channels),
                    channels,
                    22_050,
                    bits,
                    float,
                );
                let d = decode(&wav, 1.0).unwrap();
                assert_eq!((d.format, d.channels, d.rate), ("wav", channels, 22_050));
                assert_eq!(d.pcm.len(), 11_025, "{bits} {float} {channels}");
                assert!((d.seconds - 0.5).abs() < 1e-9);
                let peak = d.pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
                // 0.5 full scale; 8-bit is coarse.
                assert!(peak.abs_diff(16_384) < 300, "{bits} {float} peak {peak}");
                assert!((frequency(&d.pcm) - 440.0).abs() < 4.0, "{bits} {float}");
            }
        }
    }

    #[test]
    fn resampling_keeps_pitch_and_duration() {
        for rate in [8_000, 11_025, 32_000, 44_100, 48_000, 96_000] {
            let wav = encode_wav(&sine(330.0, rate, 1.0, 2), 2, rate, 16, false);
            let d = decode(&wav, 1.0).unwrap();
            assert_eq!(d.rate, rate);
            assert_eq!(d.pcm.len(), output_len(u64::from(rate), rate));
            // Upsampling stops at the last source sample (up to ~3 samples short at 8 kHz).
            assert!(d.pcm.len().abs_diff(22_050) <= 3, "{rate}: {}", d.pcm.len());
            assert!(
                (frequency(&d.pcm) - 330.0).abs() < 3.0,
                "{rate}: {}",
                frequency(&d.pcm)
            );
            let peak = d.pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
            assert!(peak > 15_000 && peak < 16_500, "{rate}: peak {peak}");
        }
        // Gain scales; an opposite-phase stereo pair downmixes to silence.
        let wav = encode_wav(&sine(330.0, 22_050, 0.1, 1), 1, 22_050, 16, false);
        let half = decode(&wav, 0.5).unwrap();
        let peak = half.pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!(peak.abs_diff(8_192) < 40, "{peak}");
        let mut opposite = sine(330.0, 22_050, 0.1, 2);
        opposite
            .iter_mut()
            .skip(1)
            .step_by(2)
            .for_each(|s| *s = -*s);
        let d = decode(&encode_wav(&opposite, 2, 22_050, 16, false), 1.0).unwrap();
        assert!(d.pcm.iter().all(|v| v.unsigned_abs() <= 1));
    }

    #[test]
    fn corrupt_and_oversized_files_are_errors() {
        let good = encode_wav(&sine(440.0, 22_050, 0.1, 1), 1, 22_050, 16, false);
        let err = |b: &[u8]| format!("{:#}", decode(b, 1.0).unwrap_err());
        assert!(err(b"hello world, not audio").contains("not a WAV"));
        assert!(
            err(&good[..good.len() - 10]).contains("truncated"),
            "{}",
            err(&good[..good.len() - 10])
        );
        let mut no_data = good[..36].to_vec();
        no_data[4..8].copy_from_slice(&28u32.to_le_bytes());
        assert!(err(&no_data).contains("no data chunk"));
        let mut adpcm = good.clone();
        adpcm[20] = 2;
        assert!(err(&adpcm).contains("format tag 2"));
        let mut six = good.clone();
        six[22] = 6;
        assert!(err(&six).contains("6 channels"));
        let mut slow = good.clone();
        slow[24..28].copy_from_slice(&100u32.to_le_bytes());
        assert!(err(&slow).contains("sample rate 100"));
        let mut align = good.clone();
        align[32] = 4;
        assert!(err(&align).contains("block align"));
        // 121 s of 8 kHz mono: rejected from the header, before decoding.
        let long = encode_wav(&vec![0.0; 8_000 * 121], 1, 8_000, 8, false);
        assert!(err(&long).contains("budget 120 s"), "{}", err(&long));
        let huge = vec![0u8; MAX_FILE_BYTES as usize + 1];
        assert!(err(&huge).contains("budget"));
        assert!(err(b"OggS\0\x02 garbage that is not vorbis").contains("Ogg Vorbis header"));
    }

    #[test]
    fn odd_sized_chunks_are_padded_and_unknown_chunks_skipped() {
        let good = encode_wav(&sine(440.0, 22_050, 0.1, 1), 1, 22_050, 16, false);
        let mut with_list = good[..36].to_vec();
        with_list.extend_from_slice(b"LIST");
        with_list.extend_from_slice(&3u32.to_le_bytes());
        with_list.extend_from_slice(b"abc\0");
        with_list.extend_from_slice(&good[36..]);
        let total = (with_list.len() - 8) as u32;
        with_list[4..8].copy_from_slice(&total.to_le_bytes());
        let d = decode(&with_list, 1.0).unwrap();
        assert_eq!(d.pcm, decode(&good, 1.0).unwrap().pcm);
    }

    #[test]
    fn the_first_party_ogg_fixture_decodes() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/audio-lab/sounds/chime.ogg"
        ))
        .unwrap();
        let d = decode(&bytes, 1.0).unwrap();
        assert_eq!((d.format, d.channels, d.rate), ("ogg", 2, 44_100));
        assert!((d.seconds - 2.0).abs() < 0.05, "{}", d.seconds);
        assert!(d.pcm.len().abs_diff(44_100) < 1_200, "{}", d.pcm.len());
        let peak = d.pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!(peak > 3_000, "{peak}");
        // A damaged middle is an error or a shorter stream, never a panic.
        let mut damaged = bytes.clone();
        let middle = damaged.len() / 2;
        damaged[middle..middle + 64].fill(0x55);
        if let Ok(d) = decode(&damaged, 1.0) {
            assert!(d.seconds <= 2.05);
        }
        if let Ok(d) = decode(&bytes[..bytes.len() / 3], 1.0) {
            assert!(d.seconds < 2.0, "{}", d.seconds);
        }
    }
}
