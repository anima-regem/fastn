#[derive(Debug, Clone, Copy)]
pub enum Event {
    // FocusGained,
    // FocusLost,
    // Key { code: u32, pressed: bool },
    // Mouse { x: u32, y: u32, pressed: bool },
    // Resize(u16, u16),
    OnMouseEnter,
    OnMouseLeave,
    CursorMoved {
        x: f64,
        y: f64,
    },
    NoOp,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum EventKind {
    OnMouseEnter,
    OnMouseLeave,
    CursorMoved,
}

impl From<i32> for EventKind {
    fn from(i: i32) -> EventKind {
        match i {
            0 => EventKind::OnMouseEnter,
            1 => EventKind::OnMouseLeave,
            2 => EventKind::CursorMoved,
            _ => panic!("Unknown UIProperty: {}", i),
        }
    }
}

impl From<EventKind> for i32 {
    fn from(v: EventKind) -> i32 {
        match v {
            EventKind::OnMouseEnter => 0,
            EventKind::OnMouseLeave => 1,
            EventKind::CursorMoved => 2,
        }
    }
}

impl Event {
    pub fn is_nop(&self) -> bool {
        matches!(self, Event::NoOp)
    }
}