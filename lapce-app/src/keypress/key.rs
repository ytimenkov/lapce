use floem::ui_events::keyboard::{self, Key, NamedKey};
use floem::ui_events::pointer;

use super::keymap::KeyMapKey;

#[derive(Clone, Debug)]
pub(crate) enum KeyInput {
    Keyboard {
        physical: keyboard::Code,
        logical: Key,
        location: keyboard::Location,
        key_without_modifiers: Key,
        repeat: bool,
    },
    Pointer(pointer::PointerButton),
}

impl KeyInput {
    pub fn keymap_key(&self) -> Option<KeyMapKey> {
        if let KeyInput::Keyboard {
            repeat, logical, ..
        } = self
        {
            if *repeat
                && (matches!(
                    logical,
                    Key::Named(NamedKey::Meta)
                        | Key::Named(NamedKey::Shift)
                        | Key::Named(NamedKey::Alt)
                        | Key::Named(NamedKey::Control),
                ))
            {
                return None;
            }
        }

        Some(match self {
            KeyInput::Pointer(b) => KeyMapKey::Pointer(*b),
            KeyInput::Keyboard {
                physical,
                key_without_modifiers,
                logical,
                location,
                ..
            } => {
                #[allow(clippy::single_match)]
                match location {
                    keyboard::Location::Numpad => {
                        return Some(KeyMapKey::Logical(logical.to_owned()));
                    }
                    _ => {}
                }

                match key_without_modifiers {
                    Key::Named(NamedKey::Dead | NamedKey::Unidentified) => {
                        KeyMapKey::Physical(*physical)
                    }
                    Key::Named(_) => {
                        KeyMapKey::Logical(key_without_modifiers.to_owned())
                    }
                    Key::Character(c) => {
                        if c.len() == 1 && c.is_ascii() {
                            KeyMapKey::Logical(Key::Character(c.to_lowercase()))
                        } else {
                            KeyMapKey::Physical(*physical)
                        }
                    }
                }
            }
        })
    }
}
