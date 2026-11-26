//! Writes `content/axiom-town/ui/axiom-pixel.ttf`: an original 5x7 pixel display font
//! (caps, digits, common punctuation; lower case maps to the caps) for the plan M4 content
//! font demo. Every glyph below is drawn for this project; the file is MIT like the repo.
//! Deterministic: the same bytes on every run (no timestamps).
//!
//! `cargo run -p axiom-runtime --example build_ui_font`
use std::collections::BTreeMap;

/// 7 rows of 5 columns, top to bottom; '#' = ink.
const GLYPHS: &[(char, [&str; 7])] = &[
    (
        'A',
        [
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'B',
        [
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
    ),
    (
        'C',
        [
            ".####", "#....", "#....", "#....", "#....", "#....", ".####",
        ],
    ),
    (
        'D',
        [
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ],
    ),
    (
        'E',
        [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
    ),
    (
        'F',
        [
            "#####", "#....", "#....", "####.", "#....", "#....", "#....",
        ],
    ),
    (
        'G',
        [
            ".####", "#....", "#....", "#.###", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'H',
        [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'I',
        [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####",
        ],
    ),
    (
        'J',
        [
            "..###", "....#", "....#", "....#", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'K',
        [
            "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
        ],
    ),
    (
        'L',
        [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
    ),
    (
        'M',
        [
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'N',
        [
            "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'O',
        [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'P',
        [
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
    ),
    (
        'Q',
        [
            ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#",
        ],
    ),
    (
        'R',
        [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
    ),
    (
        'S',
        [
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
    ),
    (
        'T',
        [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
    ),
    (
        'U',
        [
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'V',
        [
            "#...#", "#...#", "#...#", "#...#", ".#.#.", ".#.#.", "..#..",
        ],
    ),
    (
        'W',
        [
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ],
    ),
    (
        'X',
        [
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ],
    ),
    (
        'Y',
        [
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
    ),
    (
        'Z',
        [
            "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####",
        ],
    ),
    (
        '0',
        [
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
    ),
    (
        '1',
        [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
    ),
    (
        '2',
        [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
    ),
    (
        '3',
        [
            "####.", "....#", "....#", ".###.", "....#", "....#", "####.",
        ],
    ),
    (
        '4',
        [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
    ),
    (
        '5',
        [
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ],
    ),
    (
        '6',
        [
            ".###.", "#....", "#....", "####.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '7',
        [
            "#####", "....#", "...#.", "..#..", "..#..", "..#..", "..#..",
        ],
    ),
    (
        '8',
        [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '9',
        [
            ".###.", "#...#", "#...#", ".####", "....#", "....#", ".###.",
        ],
    ),
    (
        '!',
        [
            "..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..",
        ],
    ),
    (
        '?',
        [
            ".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..",
        ],
    ),
    (
        '.',
        [
            ".....", ".....", ".....", ".....", ".....", ".....", "..#..",
        ],
    ),
    (
        ',',
        [
            ".....", ".....", ".....", ".....", ".....", "..#..", ".#...",
        ],
    ),
    (
        ':',
        [
            ".....", "..#..", ".....", ".....", ".....", "..#..", ".....",
        ],
    ),
    (
        '\'',
        [
            "..#..", "..#..", ".....", ".....", ".....", ".....", ".....",
        ],
    ),
    (
        '-',
        [
            ".....", ".....", ".....", ".###.", ".....", ".....", ".....",
        ],
    ),
    (
        '+',
        [
            ".....", "..#..", "..#..", "#####", "..#..", "..#..", ".....",
        ],
    ),
    (
        '/',
        [
            "....#", "...#.", "...#.", "..#..", ".#...", ".#...", "#....",
        ],
    ),
    (
        '(',
        [
            "...#.", "..#..", ".#...", ".#...", ".#...", "..#..", "...#.",
        ],
    ),
    (
        ')',
        [
            ".#...", "..#..", "...#.", "...#.", "...#.", "..#..", ".#...",
        ],
    ),
    (
        '_',
        [
            ".....", ".....", ".....", ".....", ".....", ".....", "#####",
        ],
    ),
    (
        '\u{b7}',
        [
            ".....", ".....", ".....", "..#..", ".....", ".....", ".....",
        ],
    ),
];

const UNIT: i16 = 100; // one pixel in font units (1000 per em)
const ADVANCE: u16 = 600; // 5 pixels + 1 pixel spacing
const SPACE: u16 = 400;
const ASCENT: i16 = 800;
const DESCENT: i16 = -200;

struct Glyph {
    /// Rectangles (x0, y0, x1, y1) in font units, y up from the baseline.
    rects: Vec<[i16; 4]>,
    advance: u16,
}

fn glyph(rows: &[&str; 7]) -> Glyph {
    let mut rects = vec![];
    for (r, row) in rows.iter().enumerate() {
        let top = (7 - r as i16) * UNIT;
        let cells: Vec<bool> = row.chars().map(|c| c == '#').collect();
        let mut c = 0;
        while c < cells.len() {
            if !cells[c] {
                c += 1;
                continue;
            }
            let start = c;
            while c < cells.len() && cells[c] {
                c += 1;
            }
            rects.push([start as i16 * UNIT, top - UNIT, c as i16 * UNIT, top]);
        }
    }
    Glyph {
        rects,
        advance: ADVANCE,
    }
}

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut word = [0u8; 4];
        word[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// `glyf` record of one glyph: each rectangle is a clockwise contour of 4 on-curve points.
fn glyf_record(g: &Glyph) -> Vec<u8> {
    let mut out = vec![];
    if g.rects.is_empty() {
        return out;
    }
    let x_min = g.rects.iter().map(|r| r[0]).min().unwrap();
    let y_min = g.rects.iter().map(|r| r[1]).min().unwrap();
    let x_max = g.rects.iter().map(|r| r[2]).max().unwrap();
    let y_max = g.rects.iter().map(|r| r[3]).max().unwrap();
    be16(&mut out, g.rects.len() as u16);
    for v in [x_min, y_min, x_max, y_max] {
        be16(&mut out, v as u16);
    }
    for i in 0..g.rects.len() {
        be16(&mut out, (i * 4 + 3) as u16);
    }
    be16(&mut out, 0); // no instructions
    let points: Vec<(i16, i16)> = g
        .rects
        .iter()
        .flat_map(|&[x0, y0, x1, y1]| [(x0, y0), (x0, y1), (x1, y1), (x1, y0)])
        .collect();
    out.extend(std::iter::repeat_n(0x01u8, points.len())); // on curve, 16-bit deltas
    let (mut px, mut py) = (0i16, 0i16);
    for &(x, _) in &points {
        be16(&mut out, (x - px) as u16);
        px = x;
    }
    for &(_, y) in &points {
        be16(&mut out, (y - py) as u16);
        py = y;
    }
    out
}

fn name_table() -> Vec<u8> {
    let records: [(u16, &str); 7] = [
        (0, "Copyright (c) 2026 Axiom Engine contributors"),
        (1, "Axiom Pixel"),
        (2, "Regular"),
        (4, "Axiom Pixel Regular"),
        (5, "Version 1.000"),
        (6, "AxiomPixel-Regular"),
        (13, "MIT License"),
    ];
    let mut strings = vec![];
    let mut out = vec![];
    be16(&mut out, 0);
    be16(&mut out, records.len() as u16);
    be16(&mut out, (6 + 12 * records.len()) as u16);
    for (id, text) in records {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        for v in [3, 1, 0x409, id, utf16.len() as u16, strings.len() as u16] {
            be16(&mut out, v);
        }
        strings.extend(utf16);
    }
    out.extend(strings);
    out
}

fn main() {
    // Glyph order: .notdef, space, then the drawn glyphs.
    let mut glyphs = vec![
        Glyph {
            rects: vec![
                [0, 0, 500, 100],
                [0, 600, 500, 700],
                [0, 100, 100, 600],
                [400, 100, 500, 600],
            ],
            advance: ADVANCE,
        },
        Glyph {
            rects: vec![],
            advance: SPACE,
        },
    ];
    let mut cmap: BTreeMap<u32, u16> = BTreeMap::new();
    cmap.insert(' ' as u32, 1);
    for (c, rows) in GLYPHS {
        let index = glyphs.len() as u16;
        glyphs.push(glyph(rows));
        cmap.insert(*c as u32, index);
        if c.is_ascii_uppercase() {
            cmap.insert(c.to_ascii_lowercase() as u32, index);
        }
    }
    let n = glyphs.len() as u16;

    // glyf + loca (long offsets)
    let mut glyf = vec![];
    let mut loca = vec![];
    for g in &glyphs {
        be32(&mut loca, glyf.len() as u32);
        glyf.extend(glyf_record(g));
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
    }
    be32(&mut loca, glyf.len() as u32);

    // cmap: format 4, one segment per code point plus the 0xFFFF terminator.
    let codes: Vec<(u16, u16)> = cmap.iter().map(|(&c, &g)| (c as u16, g)).collect();
    let seg = codes.len() as u16 + 1;
    let mut sub = vec![];
    let length = 16 + 8 * seg;
    let search = 2 * (1u16 << (15 - seg.leading_zeros() as u16).min(15));
    for v in [
        4,
        length,
        0,
        seg * 2,
        search,
        search.trailing_zeros() as u16 - 1,
        seg * 2 - search,
    ] {
        be16(&mut sub, v);
    }
    for &(c, _) in &codes {
        be16(&mut sub, c);
    }
    be16(&mut sub, 0xFFFF);
    be16(&mut sub, 0);
    for &(c, _) in &codes {
        be16(&mut sub, c);
    }
    be16(&mut sub, 0xFFFF);
    for &(c, g) in &codes {
        be16(&mut sub, g.wrapping_sub(c));
    }
    be16(&mut sub, 1);
    for _ in 0..seg {
        be16(&mut sub, 0);
    }
    let mut cmap_table = vec![];
    for v in [0u16, 1, 3, 1] {
        be16(&mut cmap_table, v);
    }
    be32(&mut cmap_table, 12);
    cmap_table.extend(sub);

    let max_points = glyphs.iter().map(|g| g.rects.len() * 4).max().unwrap() as u16;
    let max_contours = glyphs.iter().map(|g| g.rects.len()).max().unwrap() as u16;
    let all = glyphs.iter().flat_map(|g| &g.rects);
    let x_max = all.clone().map(|r| r[2]).max().unwrap();
    let y_max = all.clone().map(|r| r[3]).max().unwrap();

    let mut head = vec![];
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0); // checkSumAdjustment, patched below
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0x000B);
    be16(&mut head, 1000);
    head.extend([0u8; 16]); // created, modified: fixed for deterministic output
    for v in [0i16, 0, x_max, y_max] {
        be16(&mut head, v as u16);
    }
    for v in [0u16, 8, 2, 1, 0] {
        be16(&mut head, v);
    }

    let mut hhea = vec![];
    be32(&mut hhea, 0x0001_0000);
    for v in [ASCENT, DESCENT, 0] {
        be16(&mut hhea, v as u16);
    }
    be16(&mut hhea, ADVANCE);
    for v in [0i16, 0, x_max, 1, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut hhea, v as u16);
    }
    be16(&mut hhea, n);

    let mut hmtx = vec![];
    for g in &glyphs {
        be16(&mut hmtx, g.advance);
        be16(
            &mut hmtx,
            g.rects.iter().map(|r| r[0]).min().unwrap_or(0) as u16,
        );
    }

    let mut maxp = vec![];
    be32(&mut maxp, 0x0001_0000);
    for v in [n, max_points, max_contours, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, v);
    }

    let mut post = vec![];
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0);
    be16(&mut post, (-100i16) as u16);
    be16(&mut post, 50);
    for _ in 0..5 {
        be32(&mut post, 0);
    }

    let mut os2 = vec![];
    for v in [4u16, ADVANCE, 400, 5, 0] {
        be16(&mut os2, v);
    }
    for v in [500i16, 500, 0, 140, 500, 500, 0, 480, 50, 250, 0] {
        be16(&mut os2, v as u16);
    }
    os2.extend([0u8; 10]); // panose
    be32(&mut os2, 0b11); // Basic Latin + Latin-1 Supplement
    be32(&mut os2, 0);
    be32(&mut os2, 0);
    be32(&mut os2, 0);
    os2.extend(*b"AXIM");
    be16(&mut os2, 0x40); // regular
    be16(&mut os2, 0x20);
    be16(&mut os2, 0xB7);
    for v in [ASCENT, DESCENT, 0] {
        be16(&mut os2, v as u16);
    }
    be16(&mut os2, ASCENT as u16);
    be16(&mut os2, (-DESCENT) as u16);
    be32(&mut os2, 1);
    be32(&mut os2, 0);
    for v in [500u16, 700, 0, 0x20, 1] {
        be16(&mut os2, v);
    }

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap_table),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name_table()),
        (b"post", post),
    ];
    tables.sort_by_key(|(tag, _)| **tag);
    let count = tables.len() as u16;
    let pow = 1u16 << (15 - count.leading_zeros() as u16);
    let mut font = vec![];
    be32(&mut font, 0x0001_0000);
    for v in [
        count,
        pow * 16,
        pow.trailing_zeros() as u16,
        count * 16 - pow * 16,
    ] {
        be16(&mut font, v);
    }
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = 0;
    let mut body = vec![];
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        be32(&mut font, checksum(data));
        be32(&mut font, offset as u32);
        be32(&mut font, data.len() as u32);
        if *tag == b"head" {
            head_at = offset;
        }
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables.len() + body.len();
    }
    font.extend(body);
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
    font[head_at + 8..head_at + 12].copy_from_slice(&adjust.to_be_bytes());

    ttf_parser::Face::parse(&font, 0).expect("the generated font parses");
    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../content/axiom-town/ui/axiom-pixel.ttf"
    );
    std::fs::write(out, &font).unwrap();
    println!("{out}: {} bytes, {n} glyphs", font.len());
}
