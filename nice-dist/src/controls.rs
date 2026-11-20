//! Saved circuit selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    #[default]
    Ts808 = 0,
    Ts9 = 1,
    Sd1 = 2,
    Od1 = 3,
    Rat = 4,
    Clone = 5,
}
impl Variant {
    pub const ALL: [Self; 6] = [
        Self::Ts808,
        Self::Ts9,
        Self::Od1,
        Self::Sd1,
        Self::Rat,
        Self::Clone,
    ];
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Ts9,
            2 => Self::Sd1,
            3 => Self::Od1,
            4 => Self::Rat,
            5 => Self::Clone,
            _ => Self::Ts808,
        }
    }
    pub fn index(self) -> i32 {
        self as i32
    }
    pub fn from_number(number: u32) -> Self {
        match number {
            2 => Self::Ts9,
            3 => Self::Sd1,
            4 => Self::Od1,
            5 => Self::Rat,
            6 => Self::Clone,
            _ => Self::Ts808,
        }
    }
    pub fn number(self) -> u32 {
        self.index() as u32 + 1
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ts808 => "TS808",
            Self::Ts9 => "TS9",
            Self::Sd1 => "SD-1",
            Self::Od1 => "OD-1",
            Self::Rat => "RAT",
            Self::Clone => "Nice Clone",
        }
    }
}
