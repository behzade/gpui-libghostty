pub mod terminal {
    use crate::{terminal_core, ui};
    terminal_core::bind_gpui!(self::ui);
}

pub mod editor {
    use crate::editor_core;
    editor_core::bind_gpui!(crate::ui);
}

#[test]
fn components_use_the_callers_gpui_types() {
    fn assert_render<T: crate::ui::Render>() {}
    assert_render::<terminal::Terminal>();
    assert_render::<editor::Terminal>();
    assert_render::<editor::NvimEditor>();

    // Check the entry points without creating a native window.
    let _ = terminal::Terminal::spawn::<()>;
    let _ = editor::NvimEditor::spawn::<()>;
}
