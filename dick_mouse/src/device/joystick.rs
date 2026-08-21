#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Joystick {
    x: i16,
    y: i16,
    center_x: u16,
    center_y: u16,
}

impl Joystick {
    pub const fn new(center_x: u16, center_y: u16) -> Self {
        Self {
            x: 0,
            y: 0,
            center_x,
            center_y,
        }
    }

    pub fn update(self, raw_x: u16, raw_y: u16) -> Self {
        Self {
            x: Self::axis(raw_x, self.center_x),
            y: Self::axis(raw_y, self.center_y),
            ..self
        }
    }

    pub const fn x(&self) -> i16 {
        self.x
    }

    pub const fn y(&self) -> i16 {
        self.y
    }

    fn axis(raw: u16, center: u16) -> i16 {
        i32::from(raw)
            .saturating_sub(i32::from(center))
            .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
    }
}
