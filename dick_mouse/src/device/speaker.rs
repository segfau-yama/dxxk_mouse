#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Speaker<const N: usize> {
    buffer: [i16; N],
}

impl<const N: usize> Speaker<N> {
    pub const fn new(buffer: [i16; N]) -> Self {
        Self { buffer }
    }

    pub const fn update(self, buffer: [i16; N]) -> Self {
        Self { buffer }
    }

    pub const fn buffer(&self) -> &[i16; N] {
        &self.buffer
    }
}
