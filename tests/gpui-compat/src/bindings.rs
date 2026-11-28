pub mod terminal {
    use crate::{terminal_core, ui};
    terminal_core::bind_gpui!(self::ui);
}

#[test]
fn component_uses_the_callers_gpui_types() {
    fn assert_render<T: crate::ui::Render>() {}
    assert_render::<terminal::Terminal>();

    // Check the entry point without creating a native window.
    let _ = terminal::Terminal::spawn::<()>;
}
