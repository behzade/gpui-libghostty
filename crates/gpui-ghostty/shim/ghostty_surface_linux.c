#include <dlfcn.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include <ghostty.h>

typedef bool (*gpui_ghostty_make_current_cb)(void *userdata);
typedef void (*gpui_ghostty_context_cb)(void *userdata);
typedef void (*gpui_ghostty_wakeup_cb)(void *userdata);

typedef struct gpui_ghostty_surface {
    ghostty_config_t config;
    ghostty_app_t app;
    ghostty_surface_t surface;
    void *platform_userdata;
    gpui_ghostty_make_current_cb make_current;
    gpui_ghostty_context_cb clear_current;
    gpui_ghostty_context_cb swap_buffers;
    void *wakeup_userdata;
    gpui_ghostty_wakeup_cb wakeup;
    void *clipboard_request;
    ghostty_clipboard_e clipboard_location;
    char *clipboard_write;
    ghostty_clipboard_e clipboard_write_location;
    _Atomic bool alive;
} gpui_ghostty_surface;

static pthread_once_t ghostty_once = PTHREAD_ONCE_INIT;
static int ghostty_init_result = -1;
static void *egl_library;
static void *gl_library;
typedef void (*egl_proc)(void);
typedef egl_proc (*egl_get_proc_address_fn)(const char *name);
static egl_get_proc_address_fn egl_get_proc_address;

static void initialize_ghostty(void) {
    setenv("GHOSTTY_LOG", "stderr", 0);
    ghostty_init_result = ghostty_init(0, NULL);
    egl_library = dlopen("libEGL.so.1", RTLD_LAZY | RTLD_LOCAL);
    gl_library = dlopen("libGL.so.1", RTLD_LAZY | RTLD_LOCAL);
    if (egl_library != NULL) {
        void *symbol = dlsym(egl_library, "eglGetProcAddress");
        memcpy(&egl_get_proc_address, &symbol, sizeof(egl_get_proc_address));
    }
}

static ghostty_gl_proc_t opengl_get_proc_address(const char *name) {
    egl_proc proc = egl_get_proc_address != NULL ? egl_get_proc_address(name) : NULL;
    if (proc == NULL && gl_library != NULL) {
        void *symbol = dlsym(gl_library, name);
        memcpy(&proc, &symbol, sizeof(proc));
    }
    ghostty_gl_proc_t result = NULL;
    memcpy(&result, &proc, sizeof(result));
    return result;
}

static void runtime_wakeup(void *userdata) {
    gpui_ghostty_surface *state = userdata;
    state->wakeup(state->wakeup_userdata);
}

static bool runtime_action(ghostty_app_t app, ghostty_target_s target, ghostty_action_s action) {
    (void)app;
    if (action.tag != GHOSTTY_ACTION_RENDER ||
        target.tag != GHOSTTY_TARGET_SURFACE ||
        target.target.surface == NULL) {
        return false;
    }

    gpui_ghostty_surface *state = ghostty_surface_userdata(target.target.surface);
    if (state == NULL || !state->make_current(state->platform_userdata)) return false;
    ghostty_surface_draw(target.target.surface);
    state->swap_buffers(state->platform_userdata);
    state->clear_current(state->platform_userdata);
    return true;
}

static bool runtime_read_clipboard(void *userdata, ghostty_clipboard_e location, void *request) {
    gpui_ghostty_surface *state = userdata;
    if (state->clipboard_request != NULL) return false;
    state->clipboard_request = request;
    state->clipboard_location = location;
    runtime_wakeup(state);
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
    (void)confirm;
    gpui_ghostty_surface *state = userdata;
    for (size_t index = 0; index < count; index++) {
        if (strcmp(content[index].mime, "text/plain") != 0) continue;
        char *copy = strdup(content[index].data);
        if (copy == NULL) return;
        free(state->clipboard_write);
        state->clipboard_write = copy;
        state->clipboard_write_location = location;
        runtime_wakeup(state);
        return;
    }
}

static void runtime_close_surface(void *userdata, bool process_alive) {
    (void)process_alive;
    gpui_ghostty_surface *state = userdata;
    atomic_store_explicit(&state->alive, false, memory_order_release);
    runtime_wakeup(state);
}

gpui_ghostty_surface *gpui_ghostty_surface_linux_new(
    void *platform_userdata,
    gpui_ghostty_make_current_cb make_current,
    gpui_ghostty_context_cb clear_current,
    gpui_ghostty_context_cb swap_buffers,
    const char *working_directory,
    const char *command,
    double scale_factor,
    void *wakeup_userdata,
    gpui_ghostty_wakeup_cb wakeup
) {
    pthread_once(&ghostty_once, initialize_ghostty);
    if (ghostty_init_result != GHOSTTY_SUCCESS || platform_userdata == NULL ||
        make_current == NULL || clear_current == NULL || swap_buffers == NULL ||
        wakeup_userdata == NULL || wakeup == NULL || !make_current(platform_userdata)) {
        return NULL;
    }

    gpui_ghostty_surface *state = calloc(1, sizeof(gpui_ghostty_surface));
    if (state == NULL) {
        clear_current(platform_userdata);
        return NULL;
    }
    atomic_init(&state->alive, true);
    state->platform_userdata = platform_userdata;
    state->make_current = make_current;
    state->clear_current = clear_current;
    state->swap_buffers = swap_buffers;
    state->wakeup_userdata = wakeup_userdata;
    state->wakeup = wakeup;

    state->config = ghostty_config_new();
    if (state->config == NULL) goto fail;
    ghostty_config_finalize(state->config);

    ghostty_runtime_config_s runtime = {
        .userdata = state,
        .supports_selection_clipboard = true,
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
    surface_config.platform_tag = GHOSTTY_PLATFORM_OPENGL;
    surface_config.platform.opengl.get_proc_address = opengl_get_proc_address;
    surface_config.userdata = state;
    surface_config.scale_factor = scale_factor;
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
    clear_current(platform_userdata);
    return state;

fail:
    if (state->surface != NULL) ghostty_surface_free(state->surface);
    if (state->app != NULL) ghostty_app_free(state->app);
    if (state->config != NULL) ghostty_config_free(state->config);
    clear_current(platform_userdata);
    free(state);
    return NULL;
}

void gpui_ghostty_surface_linux_free(gpui_ghostty_surface *state) {
    if (state == NULL) return;
    bool current = state->make_current(state->platform_userdata);
    if (state->surface != NULL) ghostty_surface_free(state->surface);
    if (state->app != NULL) ghostty_app_free(state->app);
    if (state->config != NULL) ghostty_config_free(state->config);
    free(state->clipboard_write);
    if (current) state->clear_current(state->platform_userdata);
    free(state);
}

void gpui_ghostty_surface_linux_tick(gpui_ghostty_surface *state) {
    if (state == NULL || state->app == NULL) return;
    ghostty_app_tick(state->app);
}

bool gpui_ghostty_surface_linux_is_alive(const gpui_ghostty_surface *state) {
    return state != NULL && atomic_load_explicit(&state->alive, memory_order_acquire) &&
        !ghostty_surface_process_exited(state->surface);
}

void *gpui_ghostty_surface_linux_take_clipboard_read(
    gpui_ghostty_surface *state,
    bool *selection
) {
    if (state == NULL || state->clipboard_request == NULL) return NULL;
    void *request = state->clipboard_request;
    state->clipboard_request = NULL;
    *selection = state->clipboard_location == GHOSTTY_CLIPBOARD_SELECTION;
    return request;
}

void gpui_ghostty_surface_linux_complete_clipboard_read(
    gpui_ghostty_surface *state,
    void *request,
    const char *text
) {
    if (state != NULL && state->surface != NULL && request != NULL && text != NULL) {
        ghostty_surface_complete_clipboard_request(state->surface, text, request, false);
    }
}

char *gpui_ghostty_surface_linux_take_clipboard_write(
    gpui_ghostty_surface *state,
    bool *selection
) {
    if (state == NULL || state->clipboard_write == NULL) return NULL;
    char *text = state->clipboard_write;
    state->clipboard_write = NULL;
    *selection = state->clipboard_write_location == GHOSTTY_CLIPBOARD_SELECTION;
    return text;
}

void gpui_ghostty_surface_linux_free_clipboard_write(char *text) {
    free(text);
}

void gpui_ghostty_surface_linux_set_size(
    gpui_ghostty_surface *state,
    uint32_t width,
    uint32_t height,
    double scale_factor
) {
    if (state == NULL || state->surface == NULL || width == 0 || height == 0) return;
    ghostty_surface_set_content_scale(state->surface, scale_factor, scale_factor);
    ghostty_surface_set_size(state->surface, width, height);
    ghostty_surface_set_occlusion(state->surface, true);
    ghostty_surface_refresh(state->surface);
}

void gpui_ghostty_surface_linux_set_visible(gpui_ghostty_surface *state, bool visible) {
    if (state == NULL || state->surface == NULL) return;
    ghostty_surface_set_occlusion(state->surface, visible);
    if (visible) ghostty_surface_refresh(state->surface);
}

void gpui_ghostty_surface_linux_set_focus(gpui_ghostty_surface *state, bool focused) {
    if (state == NULL || state->surface == NULL) return;
    ghostty_app_set_focus(state->app, focused);
    ghostty_surface_set_focus(state->surface, focused);
}

bool gpui_ghostty_surface_linux_key(
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

void gpui_ghostty_surface_linux_text(
    gpui_ghostty_surface *state,
    const char *text,
    size_t length
) {
    if (state != NULL && state->surface != NULL) ghostty_surface_text(state->surface, text, length);
}

void gpui_ghostty_surface_linux_mouse_position(
    gpui_ghostty_surface *state,
    double x,
    double y,
    int modifiers
) {
    if (state != NULL && state->surface != NULL) {
        ghostty_surface_mouse_pos(state->surface, x, y, (ghostty_input_mods_e)modifiers);
    }
}

void gpui_ghostty_surface_linux_mouse_button(
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

void gpui_ghostty_surface_linux_mouse_scroll(
    gpui_ghostty_surface *state,
    double x,
    double y,
    int modifiers
) {
    if (state != NULL && state->surface != NULL) {
        ghostty_surface_mouse_scroll(state->surface, x, y, modifiers);
    }
}
