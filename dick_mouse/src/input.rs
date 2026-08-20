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

#[cfg(test)]
mod tests {
    use super::Joystick;

    #[test]
    fn joystickはxyを保持する() {
        let joystick = Joystick::new(-10, 12);

        assert_eq!(joystick.x(), -10);
        assert_eq!(joystick.y(), 12);
    }
}
