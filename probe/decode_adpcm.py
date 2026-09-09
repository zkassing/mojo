#!/usr/bin/env python3
"""
把 gattprobe --dump-raw 落盘的 ATVV 原始帧解成 wav。

实测结论（小米蓝牙语音遥控器 VID 0x2717 PID 0x32B8，固件 2671）：
  - notification 每包 120 字节，**全部是 IMA/DVI ADPCM 4-bit 数据，没有帧头**
  - 握手 CAPS_RESP 宣称帧长 134（6 头 + 128 数据），但真实推流是裸 120 字节
  - 解码器状态（predictor / step index）**跨包连续**，不能逐包重置
  - 16 kHz / 16-bit / 单声道，每字节 2 个样本 → 120B = 240 样本 = 15 ms

用法:
  python3 decode_adpcm.py raw.bin out.wav [--rate 16000] [--swap-nibble]

--swap-nibble  先解高 4 位再解低 4 位（不同实现字节内顺序可能相反，
               若解出来全是噪声可以试这个）
"""
import sys, struct, wave

# IMA ADPCM 标准表
STEP_TABLE = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37,
    41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173,
    190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658,
    724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066,
    2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894,
    6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289,
    16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
]
INDEX_TABLE = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8]


def clamp(v, lo, hi):
    return lo if v < lo else (hi if v > hi else v)


class ADPCMDecoder:
    """状态跨包连续的 IMA ADPCM 解码器"""

    def __init__(self):
        self.pred = 0
        self.index = 0

    def decode_nibble(self, code):
        step = STEP_TABLE[self.index]
        diff = step >> 3
        if code & 1:
            diff += step >> 2
        if code & 2:
            diff += step >> 1
        if code & 4:
            diff += step
        if code & 8:
            diff = -diff
        self.pred = clamp(self.pred + diff, -32768, 32767)
        self.index = clamp(self.index + INDEX_TABLE[code], 0, 88)
        return self.pred

    def decode(self, data, swap=False):
        out = []
        for byte in data:
            lo, hi = byte & 0x0F, (byte >> 4) & 0x0F
            first, second = (hi, lo) if swap else (lo, hi)
            out.append(self.decode_nibble(first))
            out.append(self.decode_nibble(second))
        return out


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    src, dst = sys.argv[1], sys.argv[2]
    rate = 16000
    swap = "--swap-nibble" in sys.argv
    if "--rate" in sys.argv:
        rate = int(sys.argv[sys.argv.index("--rate") + 1])

    raw = open(src, "rb").read()
    print(f"输入 {len(raw)} 字节")

    dec = ADPCMDecoder()
    samples = dec.decode(raw, swap=swap)

    n = len(samples)
    dur = n / rate
    peak = max(abs(s) for s in samples) if samples else 0
    rms = int((sum(s * s for s in samples) / n) ** 0.5) if n else 0
    # 过零率：判断是语音还是噪声的粗指标（语音通常 0.02~0.20）
    zc = sum(1 for i in range(1, n) if (samples[i - 1] < 0) != (samples[i] < 0))
    zcr = zc / n if n else 0

    print(f"样本 {n} ({dur:.2f}s @ {rate}Hz)  peak={peak}  rms={rms}  过零率={zcr:.3f}")
    if peak >= 32767:
        print("  ⚠️  削波：peak 顶到满量程，可能 nibble 顺序反了，试 --swap-nibble")
    if zcr > 0.35:
        print("  ⚠️  过零率过高，像白噪声而非语音，试 --swap-nibble")

    with wave.open(dst, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack(f"<{n}h", *samples))
    print(f"→ {dst}")


if __name__ == "__main__":
    main()
