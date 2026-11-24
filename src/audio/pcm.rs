use super::decoder::SampleKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Integer { container: u16, valid: u16 },
    Float32,
}

pub const PCM16: Encoding = Encoding::Integer {
    container: 16,
    valid: 16,
};
pub const PCM24: Encoding = Encoding::Integer {
    container: 24,
    valid: 24,
};
pub const PCM24_IN_32: Encoding = Encoding::Integer {
    container: 32,
    valid: 24,
};
pub const PCM32: Encoding = Encoding::Integer {
    container: 32,
    valid: 32,
};

impl Encoding {
    pub fn bits(self) -> u16 {
        match self {
            Self::Integer { container, .. } => container,
            Self::Float32 => 32,
        }
    }

    pub fn valid_bits(self) -> u16 {
        match self {
            Self::Integer { valid, .. } => valid,
            Self::Float32 => 32,
        }
    }

    pub fn width(self) -> usize {
        usize::from(self.bits() / 8)
    }

    pub fn label(self) -> &'static str {
        match self {
            PCM16 => "PCM 16-bit",
            PCM24 => "PCM 24-bit (packed)",
            PCM24_IN_32 => "PCM 24-bit in 32-bit",
            Self::Integer { .. } => "PCM 32-bit",
            Self::Float32 => "Float 32-bit",
        }
    }

    pub fn word(self, canonical: u32) -> u32 {
        match self {
            Self::Integer { container, .. } => ((canonical as i32) >> (32 - container)) as u32,
            Self::Float32 => canonical,
        }
    }

    pub fn converted(self, sample: f64, noise_lsb: f64) -> u32 {
        match self {
            Self::Float32 => (sample.clamp(-1.0, 1.0) as f32).to_bits(),
            Self::Integer { container, valid } => {
                let range = f64::from(1_u32 << (valid - 1));
                let quantized = (sample * range + noise_lsb)
                    .round()
                    .clamp(-range, range - 1.0) as i32;
                (quantized as u32) << (container - valid)
            }
        }
    }

    pub fn write(self, bytes: &mut [u8], word: u32) {
        bytes.copy_from_slice(&word.to_le_bytes()[..self.width()]);
    }
}

pub fn exact_encodings(source: SampleKind) -> &'static [Encoding] {
    match source {
        SampleKind::Integer(1..=16) => &[PCM16, PCM24, PCM24_IN_32, PCM32],
        SampleKind::Integer(17..=24) => &[PCM24, PCM24_IN_32, PCM32],
        SampleKind::Integer(25..=32) => &[PCM32],
        SampleKind::Float32 => &[Encoding::Float32],
        _ => &[],
    }
}

pub fn normalized(raw: u32, kind: Option<SampleKind>) -> f64 {
    if matches!(kind, Some(SampleKind::Integer(_))) {
        f64::from(raw as i32) / 2_147_483_648.0
    } else {
        f64::from(f32::from_bits(raw))
    }
}

pub fn random(state: &mut u32) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    f64::from(*state) / 4_294_967_296.0
}
