use ironrdp_input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use serde::Deserialize;

/// 前端发来的输入事件，坐标是远程桌面的像素坐标。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum InputEvent {
    /// code 是浏览器 KeyboardEvent.code。
    Key { code: String, down: bool },
    /// 没有扫描码的按键或输入法上屏的文字，按 Unicode 发送。
    Text { text: String },
    Move { x: u16, y: u16 },
    /// button 是浏览器 MouseEvent.button。
    Button { button: u8, down: bool },
    /// 一格滚轮是 120。
    Wheel { vertical: bool, units: i16 },
}

/// 转换成 IronRDP 的输入操作；不认识的按键和鼠标按钮被忽略。
pub fn operations(events: &[InputEvent]) -> Vec<Operation> {
    let mut ops = Vec::with_capacity(events.len());
    for e in events {
        match e {
            InputEvent::Key { code, down } => {
                if let Some(sc) = scancode(code) {
                    ops.push(if *down {
                        Operation::KeyPressed(sc)
                    } else {
                        Operation::KeyReleased(sc)
                    });
                }
            }
            InputEvent::Text { text } => {
                for ch in text.chars() {
                    ops.push(Operation::UnicodeKeyPressed(ch));
                    ops.push(Operation::UnicodeKeyReleased(ch));
                }
            }
            InputEvent::Move { x, y } => {
                ops.push(Operation::MouseMove(MousePosition { x: *x, y: *y }))
            }
            InputEvent::Button { button, down } => {
                if let Some(b) = MouseButton::from_web_button(*button) {
                    ops.push(if *down {
                        Operation::MouseButtonPressed(b)
                    } else {
                        Operation::MouseButtonReleased(b)
                    });
                }
            }
            InputEvent::Wheel { vertical, units } => {
                if *units != 0 {
                    ops.push(Operation::WheelRotations(WheelRotations {
                        is_vertical: *vertical,
                        rotation_units: *units,
                    }));
                }
            }
        }
    }
    ops
}

/// Ctrl+Alt+Del 的按下和松开。
pub fn ctrl_alt_del() -> Vec<Operation> {
    let keys = ["ControlLeft", "AltLeft", "Delete"].map(|k| scancode(k).unwrap());
    let mut ops: Vec<Operation> = keys.iter().map(|&k| Operation::KeyPressed(k)).collect();
    ops.extend(keys.iter().rev().map(|&k| Operation::KeyReleased(k)));
    ops
}

/// KeyboardEvent.code 对应的扫描码（Set 1）。
pub fn scancode(code: &str) -> Option<Scancode> {
    let (extended, sc): (bool, u8) = match code {
        "Escape" => (false, 0x01),
        "Digit1" => (false, 0x02),
        "Digit2" => (false, 0x03),
        "Digit3" => (false, 0x04),
        "Digit4" => (false, 0x05),
        "Digit5" => (false, 0x06),
        "Digit6" => (false, 0x07),
        "Digit7" => (false, 0x08),
        "Digit8" => (false, 0x09),
        "Digit9" => (false, 0x0A),
        "Digit0" => (false, 0x0B),
        "Minus" => (false, 0x0C),
        "Equal" => (false, 0x0D),
        "Backspace" => (false, 0x0E),
        "Tab" => (false, 0x0F),
        "KeyQ" => (false, 0x10),
        "KeyW" => (false, 0x11),
        "KeyE" => (false, 0x12),
        "KeyR" => (false, 0x13),
        "KeyT" => (false, 0x14),
        "KeyY" => (false, 0x15),
        "KeyU" => (false, 0x16),
        "KeyI" => (false, 0x17),
        "KeyO" => (false, 0x18),
        "KeyP" => (false, 0x19),
        "BracketLeft" => (false, 0x1A),
        "BracketRight" => (false, 0x1B),
        "Enter" => (false, 0x1C),
        "ControlLeft" => (false, 0x1D),
        "KeyA" => (false, 0x1E),
        "KeyS" => (false, 0x1F),
        "KeyD" => (false, 0x20),
        "KeyF" => (false, 0x21),
        "KeyG" => (false, 0x22),
        "KeyH" => (false, 0x23),
        "KeyJ" => (false, 0x24),
        "KeyK" => (false, 0x25),
        "KeyL" => (false, 0x26),
        "Semicolon" => (false, 0x27),
        "Quote" => (false, 0x28),
        "Backquote" => (false, 0x29),
        "ShiftLeft" => (false, 0x2A),
        "Backslash" => (false, 0x2B),
        "KeyZ" => (false, 0x2C),
        "KeyX" => (false, 0x2D),
        "KeyC" => (false, 0x2E),
        "KeyV" => (false, 0x2F),
        "KeyB" => (false, 0x30),
        "KeyN" => (false, 0x31),
        "KeyM" => (false, 0x32),
        "Comma" => (false, 0x33),
        "Period" => (false, 0x34),
        "Slash" => (false, 0x35),
        "ShiftRight" => (false, 0x36),
        "NumpadMultiply" => (false, 0x37),
        "AltLeft" => (false, 0x38),
        "Space" => (false, 0x39),
        "CapsLock" => (false, 0x3A),
        "F1" => (false, 0x3B),
        "F2" => (false, 0x3C),
        "F3" => (false, 0x3D),
        "F4" => (false, 0x3E),
        "F5" => (false, 0x3F),
        "F6" => (false, 0x40),
        "F7" => (false, 0x41),
        "F8" => (false, 0x42),
        "F9" => (false, 0x43),
        "F10" => (false, 0x44),
        "NumLock" => (false, 0x45),
        "ScrollLock" => (false, 0x46),
        "Numpad7" => (false, 0x47),
        "Numpad8" => (false, 0x48),
        "Numpad9" => (false, 0x49),
        "NumpadSubtract" => (false, 0x4A),
        "Numpad4" => (false, 0x4B),
        "Numpad5" => (false, 0x4C),
        "Numpad6" => (false, 0x4D),
        "NumpadAdd" => (false, 0x4E),
        "Numpad1" => (false, 0x4F),
        "Numpad2" => (false, 0x50),
        "Numpad3" => (false, 0x51),
        "Numpad0" => (false, 0x52),
        "NumpadDecimal" => (false, 0x53),
        "IntlBackslash" => (false, 0x56),
        "F11" => (false, 0x57),
        "F12" => (false, 0x58),
        "NumpadEqual" => (false, 0x59),
        "F13" => (false, 0x64),
        "F14" => (false, 0x65),
        "F15" => (false, 0x66),
        "F16" => (false, 0x67),
        "F17" => (false, 0x68),
        "F18" => (false, 0x69),
        "F19" => (false, 0x6A),
        "F20" => (false, 0x6B),
        "F21" => (false, 0x6C),
        "F22" => (false, 0x6D),
        "F23" => (false, 0x6E),
        "KanaMode" => (false, 0x70),
        "Lang2" => (false, 0x71),
        "Lang1" => (false, 0x72),
        "IntlRo" => (false, 0x73),
        "F24" => (false, 0x76),
        "Convert" => (false, 0x79),
        "NonConvert" => (false, 0x7B),
        "IntlYen" => (false, 0x7D),
        "NumpadComma" => (false, 0x7E),
        "MediaTrackPrevious" => (true, 0x10),
        "MediaTrackNext" => (true, 0x19),
        "NumpadEnter" => (true, 0x1C),
        "ControlRight" => (true, 0x1D),
        "AudioVolumeMute" => (true, 0x20),
        "MediaPlayPause" => (true, 0x22),
        "MediaStop" => (true, 0x24),
        "AudioVolumeDown" => (true, 0x2E),
        "AudioVolumeUp" => (true, 0x30),
        "NumpadDivide" => (true, 0x35),
        "PrintScreen" => (true, 0x37),
        "AltRight" => (true, 0x38),
        "Home" => (true, 0x47),
        "ArrowUp" => (true, 0x48),
        "PageUp" => (true, 0x49),
        "ArrowLeft" => (true, 0x4B),
        "ArrowRight" => (true, 0x4D),
        "End" => (true, 0x4F),
        "ArrowDown" => (true, 0x50),
        "PageDown" => (true, 0x51),
        "Insert" => (true, 0x52),
        "Delete" => (true, 0x53),
        "MetaLeft" | "OSLeft" => (true, 0x5B),
        "MetaRight" | "OSRight" => (true, 0x5C),
        "ContextMenu" => (true, 0x5D),
        _ => return None,
    };
    Some(Scancode::from_u8(extended, sc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scancodes() {
        assert_eq!(scancode("KeyA").unwrap().as_u16(), 0x1E);
        assert_eq!(scancode("Enter").unwrap().as_u16(), 0x1C);
        assert_eq!(scancode("NumpadEnter").unwrap().as_u16(), 0xE01C);
        assert_eq!(scancode("MetaLeft").unwrap().as_u16(), 0xE05B);
        assert!(scancode("Fn").is_none());
    }

    #[test]
    fn parse_and_convert() {
        let events: Vec<InputEvent> = serde_json::from_str(
            r#"[{"t":"key","code":"KeyA","down":true},{"t":"text","text":"中a"},
                {"t":"move","x":10,"y":20},{"t":"button","button":0,"down":true},
                {"t":"button","button":9,"down":true},{"t":"wheel","vertical":true,"units":-120},
                {"t":"key","code":"Unknown","down":false}]"#,
        )
        .unwrap();
        let ops = operations(&events);
        // 两个字符各按下、松开一次；不认识的鼠标按钮和按键被忽略。
        assert_eq!(ops.len(), 1 + 4 + 1 + 1 + 1);
        assert!(matches!(ops[0], Operation::KeyPressed(_)));
        assert!(matches!(ops[1], Operation::UnicodeKeyPressed('中')));
        assert!(matches!(ops[5], Operation::MouseMove(MousePosition { x: 10, y: 20 })));
    }

    #[test]
    fn ctrl_alt_del_releases_in_reverse() {
        let ops = ctrl_alt_del();
        assert_eq!(ops.len(), 6);
        assert!(matches!(ops[2], Operation::KeyPressed(k) if k.as_u16() == 0xE053));
        assert!(matches!(ops[3], Operation::KeyReleased(k) if k.as_u16() == 0xE053));
    }
}
