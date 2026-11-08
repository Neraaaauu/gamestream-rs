#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub dx: i16,
    pub dy: i16,
    pub buttons: u16, // bit 0: Left, bit 1: Right, bit 2: Middle
    pub wheel: i16,
}

impl MouseEvent {
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&self.dx.to_le_bytes());
        b[2..4].copy_from_slice(&self.dy.to_le_bytes());
        b[4..6].copy_from_slice(&self.buttons.to_le_bytes());
        b[6..8].copy_from_slice(&self.wheel.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 8 {
            return None;
        }
        let dx = i16::from_le_bytes([b[0], b[1]]);
        let dy = i16::from_le_bytes([b[2], b[3]]);
        let buttons = u16::from_le_bytes([b[4], b[5]]);
        let wheel = i16::from_le_bytes([b[6], b[7]]);
        Some(Self { dx, dy, buttons, wheel })
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub scancode: u16,
    pub state: u8, // 1 = down, 0 = up
    pub _pad: u8,
}

impl KeyEvent {
    pub fn to_bytes(&self) -> [u8; 4] {
        let mut b = [0u8; 4];
        b[0..2].copy_from_slice(&self.scancode.to_le_bytes());
        b[2] = self.state;
        b[3] = 0;
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 4 {
            return None;
        }
        let scancode = u16::from_le_bytes([b[0], b[1]]);
        let state = b[2];
        Some(Self { scancode, state, _pad: 0 })
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GamepadEvent {
    pub pad_id: u8,
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
    pub _pad: u8,
}

impl GamepadEvent {
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = self.pad_id;
        b[1..3].copy_from_slice(&self.buttons.to_le_bytes());
        b[3] = self.lt;
        b[4] = self.rt;
        b[5..7].copy_from_slice(&self.lx.to_le_bytes());
        b[7..9].copy_from_slice(&self.ly.to_le_bytes());
        b[9..11].copy_from_slice(&self.rx.to_le_bytes());
        b[11..13].copy_from_slice(&self.ry.to_le_bytes());
        b[13] = 0;
        b[14] = 0;
        b[15] = 0;
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 16 {
            return None;
        }
        Some(Self {
            pad_id: b[0],
            buttons: u16::from_le_bytes([b[1], b[2]]),
            lt: b[3],
            rt: b[4],
            lx: i16::from_le_bytes([b[5], b[6]]),
            ly: i16::from_le_bytes([b[7], b[8]]),
            rx: i16::from_le_bytes([b[9], b[10]]),
            ry: i16::from_le_bytes([b[11], b[12]]),
            _pad: 0,
        })
    }
}
