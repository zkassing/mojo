//! IMA/DVI ADPCM 4-bit 解码器（→ PCM 16-bit LE, 16 kHz 单声道）。
//!
//! 与 Swift `ADPCMDecoder` 一致：
//! - 状态跨包连续，不能逐包重置
//! - nibble 顺序：**高 4 位先**（小米遥控器固件 2671 实测）

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408,
    449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066,
    2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630,
    9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];

const INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

#[derive(Debug, Default, Clone)]
pub struct AdpcmDecoder {
    pred: i32,
    index: i32,
}

impl AdpcmDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn nibble(&mut self, code: u8) -> i16 {
        let step = STEP_TABLE[self.index as usize];
        let mut diff = step >> 3;
        if code & 1 != 0 {
            diff += step >> 2;
        }
        if code & 2 != 0 {
            diff += step >> 1;
        }
        if code & 4 != 0 {
            diff += step;
        }
        if code & 8 != 0 {
            diff = -diff;
        }
        self.pred = (self.pred + diff).clamp(-32768, 32767);
        self.index = (self.index + INDEX_TABLE[code as usize]).clamp(0, 88);
        self.pred as i16
    }

    /// 解一包 ADPCM → PCM 16-bit LE（输出长度 = 输入 × 4）
    pub fn decode(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() * 4);
        for &byte in data {
            // 高 4 位先（实测）
            for code in [(byte >> 4) & 0x0F, byte & 0x0F] {
                let s = self.nibble(code);
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(pcm: &[u8]) -> Vec<i16> {
        pcm.chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn silence_stays_zero() {
        // 0x00 两个 nibble 都是 0：diff = step>>3 = 0，输出恒 0
        let mut d = AdpcmDecoder::new();
        let pcm = d.decode(&[0x00, 0x00]);
        assert_eq!(pcm.len(), 8);
        assert_eq!(samples(&pcm), vec![0, 0, 0, 0]);
    }

    #[test]
    fn known_sequence() {
        // 手算：初始 pred=0 index=0 step=7
        // nibble 0b0001: diff = 0 + 7>>2 = 1 → pred=1, index += -1 → 0
        // nibble 0b0010: diff = 0 + 7>>1 = 3 → pred=4, index=0
        // nibble 0b0100: diff = 0 + 7    = 7 → pred=11, index += 2 → 2
        // nibble 0b1000: step=STEP[2]=9, diff = -(9>>3) = -1 → pred=10
        let mut d = AdpcmDecoder::new();
        let pcm = d.decode(&[0x12, 0x48]); // 高 4 位先：1,2,4,8
        assert_eq!(samples(&pcm), vec![1, 4, 11, 10]);
    }

    #[test]
    fn state_is_continuous_across_packets() {
        // 跨包不重置：整段一次解 == 分两包解
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let mut a = AdpcmDecoder::new();
        let whole = a.decode(&data);

        let mut b = AdpcmDecoder::new();
        let mut part = b.decode(&data[..3]);
        part.extend_from_slice(&b.decode(&data[3..]));

        assert_eq!(whole, part);
    }

    #[test]
    fn full_scale_clamps() {
        // 连续最大正增量，pred 应爬到 32767 饱和而不溢出
        let mut d = AdpcmDecoder::new();
        let pcm = d.decode(&[0x77; 200]);
        let s = samples(&pcm);
        assert!(s.iter().all(|&v| v >= 0 && v <= 32767));
        assert_eq!(*s.last().unwrap(), 32767);
    }
}
