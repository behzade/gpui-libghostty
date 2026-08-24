#import <AppKit/AppKit.h>
#import <stdatomic.h>
#import <stdlib.h>
#import <string.h>
#import <ghostty.h>

@interface GpuiGhosttyView : NSView
@end

@implementation GpuiGhosttyView
- (NSView *)hitTest:(NSPoint)point {
    (void)point;
    return nil;
}
@end

typedef struct gpui_ghostty_surface {
    ghostty_config_t config;
    ghostty_app_t app;
    ghostty_surface_t surface;
    NSView *parent;
    GpuiGhosttyView *view;
    _Atomic bool alive;
    _Atomic bool needs_tick;
} gpui_ghostty_surface;

static void runtime_wakeup(void *userdata) {
    gpui_ghostty_surface *state = userdata;
    atomic_store_explicit(&state->needs_tick, true, memory_order_release);
}

static bool runtime_action(ghostty_app_t app, ghostty_target_s target, ghostty_action_s action) {
    (void)app;
    if (action.tag == GHOSTTY_ACTION_RENDER &&
        target.tag == GHOSTTY_TARGET_SURFACE &&
        target.target.surface != NULL) {
        ghostty_surface_draw(target.target.surface);
        return true;
    }
    return false;
}

static bool runtime_read_clipboard(void *userdata, ghostty_clipboard_e location, void *request) {
    (void)location;
    gpui_ghostty_surface *state = userdata;
    if (state->surface == NULL) return false;
    NSString *text = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
    if (text == nil) return false;
    ghostty_surface_complete_clipboard_request(state->surface, text.UTF8String, request, false);
    return true;
}

static void runtime_confirm_read_clipboard(
    void *userdata,
    const char *text,
    void *request,
    ghostty_clipboard_request_e kind
) {
    (void)kind;
    gpui_ghostty_surface *state = userdata;
    if (state->surface != NULL) {
        ghostty_surface_complete_clipboard_request(state->surface, text, request, true);
    }
}

static void runtime_write_clipboard(
    void *userdata,
    ghostty_clipboard_e location,
    const ghostty_clipboard_content_s *content,
    size_t count,
    bool confirm
) {
    (void)userdata;
    (void)location;
    (void)confirm;
    for (size_t index = 0; index < count; index++) {
        if (strcmp(content[index].mime, "text/plain") != 0) continue;
        NSString *text = [NSString stringWithUTF8String:content[index].data];
        if (text == nil) return;
        NSPasteboard *pasteboard = [NSPasteboard generalPasteboard];
        [pasteboard clearContents];
        [pasteboard setString:text forType:NSPasteboardTypeString];
        return;
    }
}

static void runtime_close_surface(void *userdata, bool process_alive) {
    (void)process_alive;
    gpui_ghostty_surface *state = userdata;
    atomic_store_explicit(&state->alive, false, memory_order_release);
}

gpui_ghostty_surface *gpui_ghostty_surface_new(
    void *parent_view,
    const char *working_directory,
    const char *command
) {
    static dispatch_once_t once;
    static int init_result = -1;
    dispatch_once(&once, ^{
        setenv("GHOSTTY_LOG", "stderr", 0);
        init_result = ghostty_init(0, NULL);
    });
    if (init_result != GHOSTTY_SUCCESS || parent_view == NULL) return NULL;

    gpui_ghostty_surface *state = calloc(1, sizeof(gpui_ghostty_surface));
    if (state == NULL) return NULL;
    atomic_init(&state->alive, true);
    atomic_init(&state->needs_tick, false);
    state->parent = (NSView *)parent_view;
    state->view = [[GpuiGhosttyView alloc] initWithFrame:NSMakeRect(0, 0, 800, 600)];
    [state->view setHidden:YES];
    [state->parent addSubview:state->view];

    state->config = ghostty_config_new();
    if (state->config == NULL) goto fail;
    ghostty_config_finalize(state->config);

    ghostty_runtime_config_s runtime = {
        .userdata = state,
        .supports_selection_clipboard = false,
        .wakeup_cb = runtime_wakeup,
        .action_cb = runtime_action,
        .read_clipboard_cb = runtime_read_clipboard,
        .confirm_read_clipboard_cb = runtime_confirm_read_clipboard,
        .write_clipboard_cb = runtime_write_clipboard,
        .close_surface_cb = runtime_close_surface,
    };
    state->app = ghostty_app_new(&runtime, state->config);
    if (state->app == NULL) goto fail;

    ghostty_surface_config_s surface_config = ghostty_surface_config_new();
    surface_config.platform_tag = GHOSTTY_PLATFORM_MACOS;
    surface_config.platform.macos.nsview = state->view;
    surface_config.userdata = state;
    surface_config.scale_factor = state->parent.window.backingScaleFactor ?: NSScreen.mainScreen.backingScaleFactor;
    ghostty_env_var_s environment[] = {
        { .key = "TERM", .value = "xterm-256color" },
        { .key = "COLORTERM", .value = "truecolor" },
        { .key = "TERM_PROGRAM", .value = "gpui-ghostty" },
    };
    surface_config.working_directory = working_directory;
    surface_config.command = command;
    surface_config.env_vars = environment;
    surface_config.env_var_count = sizeof(environment) / sizeof(environment[0]);
    surface_config.wait_after_command = false;
    surface_config.context = GHOSTTY_SURFACE_CONTEXT_WINDOW;
    state->surface = ghostty_surface_new(state->app, &surface_config);
    if (state->surface == NULL) goto fail;

    ghostty_app_set_focus(state->app, true);
    ghostty_surface_set_focus(state->surface, true);
    return state;

fail:
    if (state->surface != NULL) ghostty_surface_free(state->surface);
    if (state->app != NULL) ghostty_app_free(state->app);
    if (state->config != NULL) ghostty_config_free(state->config);
    [state->view removeFromSuperview];
    [state->view release];
    free(state);
    return NULL;
}

void gpui_ghostty_surface_free(gpui_ghostty_surface *state) {
    if (state == NULL) return;
    [state->view removeFromSuperview];
    if (state->surface != NULL) ghostty_surface_free(state->surface);
    if (state->app != NULL) ghostty_app_free(state->app);
    if (state->config != NULL) ghostty_config_free(state->config);
    [state->view release];
    free(state);
}

void gpui_ghostty_surface_tick(gpui_ghostty_surface *state) {
    if (state == NULL || state->app == NULL) return;
    atomic_store_explicit(&state->needs_tick, false, memory_order_release);
    ghostty_app_tick(state->app);
}

bool gpui_ghostty_surface_needs_tick(const gpui_ghostty_surface *state) {
    return state != NULL && atomic_load_explicit(&state->needs_tick, memory_order_acquire);
}

bool gpui_ghostty_surface_is_alive(const gpui_ghostty_surface *state) {
    return state != NULL && atomic_load_explicit(&state->alive, memory_order_acquire)
        && !ghostty_surface_process_exited(state->surface);
}

void gpui_ghostty_surface_set_frame(
    gpui_ghostty_surface *state,
    double x,
    double y,
    double width,
    double height
) {
    if (state == NULL || state->surface == NULL) return;
    double parent_height = NSHeight(state->parent.bounds);
    [state->view setFrame:NSMakeRect(x, parent_height - y - height, width, height)];
    [state->view setHidden:NO];
    double scale = state->parent.window.backingScaleFactor ?: NSScreen.mainScreen.backingScaleFactor;
    ghostty_surface_set_content_scale(state->surface, scale, scale);
    ghostty_surface_set_size(state->surface, (uint32_t)(width * scale), (uint32_t)(height * scale));
    ghostty_surface_set_occlusion(state->surface, true);
    ghostty_surface_refresh(state->surface);
}

void gpui_ghostty_surface_set_visible(gpui_ghostty_surface *state, bool visible) {
    if (state == NULL || state->surface == NULL) return;
    [state->view setHidden:!visible];
    ghostty_surface_set_occlusion(state->surface, visible);
    if (visible) ghostty_surface_refresh(state->surface);
}

void gpui_ghostty_surface_set_focus(gpui_ghostty_surface *state, bool focused) {
    if (state == NULL || state->surface == NULL) return;
    ghostty_app_set_focus(state->app, focused);
    ghostty_surface_set_focus(state->surface, focused);
}

bool gpui_ghostty_surface_key(
    gpui_ghostty_surface *state,
    int action,
    int modifiers,
    int consumed_modifiers,
    uint32_t keycode,
    const char *text,
    uint32_t unshifted_codepoint
) {
    if (state == NULL || state->surface == NULL) return false;
    ghostty_input_key_s event = {
        .action = (ghostty_input_action_e)action,
        .mods = (ghostty_input_mods_e)modifiers,
        .consumed_mods = (ghostty_input_mods_e)consumed_modifiers,
        .keycode = keycode,
        .text = text,
        .unshifted_codepoint = unshifted_codepoint,
        .composing = false,
    };
    return ghostty_surface_key(state->surface, event);
}

void gpui_ghostty_surface_text(gpui_ghostty_surface *state, const char *text, size_t length) {
    if (state != NULL && state->surface != NULL) ghostty_surface_text(state->surface, text, length);
}

void gpui_ghostty_surface_mouse_position(
    gpui_ghostty_surface *state,
    double x,
    double y,
    int modifiers
) {
    if (state != NULL && state->surface != NULL) {
        ghostty_surface_mouse_pos(state->surface, x, y, (ghostty_input_mods_e)modifiers);
    }
}

void gpui_ghostty_surface_mouse_button(
    gpui_ghostty_surface *state,
    int mouse_state,
    int button,
    int modifiers
) {
    if (state != NULL && state->surface != NULL) {
        ghostty_surface_mouse_button(
            state->surface,
            (ghostty_input_mouse_state_e)mouse_state,
            (ghostty_input_mouse_button_e)button,
            (ghostty_input_mods_e)modifiers
        );
    }
}

void gpui_ghostty_surface_mouse_scroll(
    gpui_ghostty_surface *state,
    double x,
    double y,
    int modifiers
) {
    if (state != NULL && state->surface != NULL) {
        ghostty_surface_mouse_scroll(state->surface, x, y, modifiers);
    }
}
