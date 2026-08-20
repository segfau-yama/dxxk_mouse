pub mod button;
pub mod encoder;

pub use button::{Button, Led, Toggle};
pub use encoder::RotaryEncoder;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Joystick {
    x: i16,
    y: i16,
}

impl Joystick {
    pub const fn new(x: i16, y: i16) -> Self {
        Self { x, y }
    }

    pub const fn x(&self) -> i16 {
        self.x
    }

    pub const fn y(&self) -> i16 {
        self.y
    }
}
