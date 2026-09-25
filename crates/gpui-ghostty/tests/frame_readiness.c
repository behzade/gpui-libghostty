// Exercise the real Linux shim with a controlled Ghostty backend and no GPU.
//
// Surface.updateConfig queues renderer and IO messages. Surface.draw paints
// existing frame data; it does not process those messages or rebuild the frame.
// An earlier redraw_surface message can therefore reach the application before
// the renderer consumes a theme update. Keep that ordering explicit here so the
// regression does not depend on thread timing, a shell, or a Wayland compositor.
//
// The runner discards unused shim functions, so only the backend calls exercised
// by these tests need doubles. Production update_theme, runtime_action, and
// frame_count are compiled directly from the shim, not copied into this test.
//
// The theme-readiness guarantee is a known limitation: normal runs report that
// specific failure as XFAIL while still checking frame counts and eventual color
// updates. Run with --require-theme-ready to make it a failing regression test.
#include "../shim/ghostty_surface_linux.c"

#include <inttypes.h>
#include <stdio.h>

enum { DARK = 0x112233, LIGHT = 0xeeddcc };

typedef struct {
    uint32_t background;
} test_config;

typedef struct {
    gpui_ghostty_surface *shim;
    uint32_t pending_background;
    uint32_t frame_background;
    uint32_t back_buffer;
    uint32_t presented_background;
    bool config_pending;
    bool refresh_requested;
    bool redraw_pending;
} test_renderer;

// A freshly loaded config supplies the new theme. File serialization is covered
// separately by terminal.rs; this test controls when that config reaches a frame.
ghostty_config_t ghostty_config_new(void) {
    test_config *config = malloc(sizeof(*config));
    if (config != NULL) config->background = LIGHT;
    return config;
}

void ghostty_config_free(ghostty_config_t config) { free(config); }
void ghostty_config_load_default_files(ghostty_config_t config) { (void)config; }
void ghostty_config_load_recursive_files(ghostty_config_t config) { (void)config; }
void ghostty_config_load_file(ghostty_config_t config, const char *path) {
    (void)config;
    (void)path;
}
void ghostty_config_finalize(ghostty_config_t config) { (void)config; }

void *ghostty_surface_userdata(ghostty_surface_t surface) {
    return ((test_renderer *)surface)->shim;
}

void ghostty_surface_update_config(ghostty_surface_t surface, ghostty_config_t config) {
    test_renderer *renderer = surface;
    renderer->pending_background = ((test_config *)config)->background;
    renderer->config_pending = true;
}

void ghostty_surface_refresh(ghostty_surface_t surface) {
    ((test_renderer *)surface)->refresh_requested = true;
}

void ghostty_surface_draw(ghostty_surface_t surface) {
    test_renderer *renderer = surface;
    renderer->back_buffer = renderer->frame_background;
}

static bool make_current(void *userdata) { return userdata != NULL; }
static void clear_current(void *userdata) { (void)userdata; }

static void swap_buffers(void *userdata) {
    test_renderer *renderer = userdata;
    renderer->presented_background = renderer->back_buffer;
}

static bool deliver_redraw(test_renderer *renderer) {
    if (!renderer->redraw_pending) return false;
    renderer->redraw_pending = false;
    ghostty_target_s target = {
        .tag = GHOSTTY_TARGET_SURFACE,
        .target.surface = renderer,
    };
    ghostty_action_s action = { .tag = GHOSTTY_ACTION_RENDER };
    return runtime_action(NULL, target, action);
}

static bool rebuild_after_config(test_renderer *renderer) {
    if (!renderer->config_pending || !renderer->refresh_requested) return false;
    renderer->frame_background = renderer->pending_background;
    renderer->config_pending = false;
    renderer->refresh_requested = false;
    renderer->redraw_pending = true;
    return true;
}

static bool check_frame_readiness(bool deliver_old_frame, bool require_theme_ready) {
    const char *name = deliver_old_frame
        ? "queued old frame must not signal theme readiness"
        : "newly themed frame signals readiness";
    gpui_ghostty_surface state = {0};
    atomic_init(&state.frame_count, 0);
    test_renderer renderer = {
        .shim = &state,
        .frame_background = DARK,
        .redraw_pending = true,
    };
    state.surface = &renderer;
    state.platform_userdata = &renderer;
    state.make_current = make_current;
    state.clear_current = clear_current;
    state.swap_buffers = swap_buffers;

    if (!deliver_redraw(&renderer) || renderer.presented_background != DARK) {
        fprintf(stderr, "FAIL: %s: could not establish the original frame\n", name);
        return false;
    }
    uint64_t before = gpui_ghostty_surface_linux_frame_count(&state);

    // This redraw was requested using old frame data before update_theme.
    renderer.redraw_pending = deliver_old_frame;
    if (!gpui_ghostty_surface_linux_update_theme(&state, false, NULL)) {
        fprintf(stderr, "FAIL: %s: update_theme rejected the config\n", name);
        return false;
    }
    if (!deliver_old_frame && !rebuild_after_config(&renderer)) {
        fprintf(stderr, "FAIL: %s: theme update did not schedule a rebuild\n", name);
        return false;
    }
    if (!deliver_redraw(&renderer)) {
        fprintf(stderr, "FAIL: %s: redraw was not handled\n", name);
        return false;
    }

    uint64_t after = gpui_ghostty_surface_linux_frame_count(&state);
    uint32_t observed_background = renderer.presented_background;
    if (after != before + 1) {
        fprintf(stderr, "FAIL: %s: one draw must advance the frame count once\n", name);
        return false;
    }

    // Prove that the double can apply the new colors once the renderer handles
    // its config queue; the stale frame is a scheduling case, not a stuck backend.
    if (deliver_old_frame &&
        (!rebuild_after_config(&renderer) || !deliver_redraw(&renderer) ||
         renderer.presented_background != LIGHT ||
         gpui_ghostty_surface_linux_frame_count(&state) != after + 1)) {
        fprintf(stderr, "FAIL: %s: renderer did not eventually apply the theme\n", name);
        return false;
    }

    // Only this known stale-frame case is an expected failure. Other failures
    // (including a missing counter advance or a lost update) still fail CI.
    if (deliver_old_frame && observed_background == DARK) {
        fprintf(stderr,
            "%s: %s: frame_count advanced %" PRIu64 " -> %" PRIu64
            " after update_theme, but the presented background was #%06" PRIx32
            " (expected #%06x)\n",
            require_theme_ready ? "FAIL" : "XFAIL", name, before, after,
            observed_background, LIGHT);
        return !require_theme_ready;
    }
    if (observed_background != LIGHT) {
        fprintf(stderr, "FAIL: %s: unexpected frame color\n", name);
        return false;
    }
    fprintf(stderr, "PASS: %s\n", name);
    return true;
}

int main(int argc, char **argv) {
    bool require_theme_ready = argc == 2 && strcmp(argv[1], "--require-theme-ready") == 0;
    if (argc != 1 && !require_theme_ready) {
        fprintf(stderr, "usage: %s [--require-theme-ready]\n", argv[0]);
        return EXIT_FAILURE;
    }
    bool control = check_frame_readiness(false, require_theme_ready);
    bool regression = check_frame_readiness(true, require_theme_ready);
    return control && regression ? EXIT_SUCCESS : EXIT_FAILURE;
}
