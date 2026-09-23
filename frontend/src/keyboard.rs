//! Winit/Chad key translation, shared by desktop and Android hardware keyboards.
//! Navigation is named-key input; produced text remains the platform's authority.
use chad::winit::{event::KeyEvent, keyboard::{Key, KeyCode, NamedKey, PhysicalKey}};

pub fn named(key: &Key) -> Option<&'static str> {
    Some(match key {
        Key::Named(NamedKey::Escape) => "Escape",
        Key::Named(NamedKey::Enter) => "Enter",
        Key::Named(NamedKey::Tab) => "Tab",
        Key::Named(NamedKey::Backspace) => "Backspace",
        Key::Named(NamedKey::Delete) => "Delete",
        Key::Named(NamedKey::ArrowLeft) => "ArrowLeft",
        Key::Named(NamedKey::ArrowRight) => "ArrowRight",
        Key::Named(NamedKey::ArrowUp) => "ArrowUp",
        Key::Named(NamedKey::ArrowDown) => "ArrowDown",
        Key::Named(NamedKey::Home) => "Home",
        Key::Named(NamedKey::End) => "End",
        Key::Named(NamedKey::PageUp) => "PageUp",
        Key::Named(NamedKey::PageDown) => "PageDown",
        _ => return None,
    })
}

pub fn shortcut(event: &KeyEvent) -> Option<&str> {
    // Prefer the active layout, then physical keys for non-Latin/named-key
    // layouts (including Wine's Ctrl+C → named volume-key translation).
    let logical = match &event.logical_key {
        Key::Character(text) => Some(text.as_str()),
        _ => event.text.as_deref(),
    };
    logical.filter(|s| matches!(*s, "a" | "A" | "c" | "C" | "v" | "V" | "x" | "X" | "y" | "Y" | "z" | "Z"))
        .or(match event.physical_key {
            PhysicalKey::Code(KeyCode::KeyA) => Some("a"),
            PhysicalKey::Code(KeyCode::KeyC) => Some("c"),
            PhysicalKey::Code(KeyCode::KeyV) => Some("v"),
            PhysicalKey::Code(KeyCode::KeyX) => Some("x"),
            PhysicalKey::Code(KeyCode::KeyY) => Some("y"),
            PhysicalKey::Code(KeyCode::KeyZ) => Some("z"),
            _ => None,
        })
}
