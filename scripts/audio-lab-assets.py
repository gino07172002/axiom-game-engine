# First-party test sounds for content/audio-lab (no third-party audio).
#
#   .tools\blender\blender-4.5.3-windows-x64\blender.exe -b --factory-startup --python scripts\audio-lab-assets.py -- OUT_DIR [SECONDS]
#
# Writes OUT_DIR/hum.wav (1 s, 22,050 Hz mono 16-bit, 110 Hz + 220 Hz: whole cycles, so it
# loops without a click), OUT_DIR/chime-src.wav (stereo 44,100 Hz float source) and
# OUT_DIR/chime.ogg (the same, encoded by Blender's FFmpeg as Ogg Vorbis). With SECONDS the
# chime lasts that long (the ~30 s decode-time measurement uses 30); default 2.
import math
import os
import struct
import sys
import wave

import bpy

args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
out = os.path.abspath(args[0] if args else ".")
seconds = float(args[1]) if len(args) > 1 else 2.0
os.makedirs(out, exist_ok=True)

# hum.wav: plain 16-bit PCM through the standard library.
with wave.open(os.path.join(out, "hum.wav"), "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(22050)
    frames = bytearray()
    for i in range(22050):
        t = i / 22050
        s = 0.35 * math.sin(2 * math.pi * 110 * t) + 0.15 * math.sin(2 * math.pi * 220 * t)
        frames += struct.pack("<h", int(round(s * 32767)))
    w.writeframes(bytes(frames))

# chime-src.wav: 32-bit float stereo (format tag 3), written by hand. Left 660 Hz, right
# 880 Hz, exponential decay restarted every second.
rate = 44100
n = int(seconds * rate)
data = bytearray()
for i in range(n):
    t = i / rate
    env = math.exp(-3.0 * (t % 1.0))
    data += struct.pack("<ff", 0.5 * env * math.sin(2 * math.pi * 660 * t),
                        0.5 * env * math.sin(2 * math.pi * 880 * t))
src = os.path.join(out, "chime-src.wav")
with open(src, "wb") as f:
    fmt = struct.pack("<HHIIHH", 3, 2, rate, rate * 8, 8, 32)
    f.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(data)) + b"WAVE")
    f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt)
    f.write(b"data" + struct.pack("<I", len(data)) + data)

# chime.ogg: a sound strip mixed down by Blender (Ogg container, Vorbis codec).
scene = bpy.context.scene
scene.render.fps = 30
scene.frame_start = 1
scene.frame_end = max(1, int(round(seconds * 30)))
scene.sequence_editor_create()
scene.sequence_editor.sequences.new_sound("chime", src, 1, 1)
bpy.ops.sound.mixdown(filepath=os.path.join(out, "chime.ogg"), check_existing=False,
                      container="OGG", codec="VORBIS", channels="STEREO",
                      mixrate=44100, bitrate=96)
print("wrote", sorted(os.listdir(out)))
