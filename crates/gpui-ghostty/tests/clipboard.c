#include <assert.h>
#include "../shim/ghostty_clipboard.h"

static int completed;
static int denied;
static int asked;
static int expected_operation;
static bool allow;
static const char *expected_text;
static bool expected_confirmed;

void ghostty_surface_complete_clipboard_request(
    ghostty_surface_t surface, const ghostty_clipboard_complete_s *value, void *request
) {
    assert(surface == (void *)1 && request == (void *)2);
    assert(value->contents_len == 1);
    assert(value->contents[0].len == strlen(expected_text));
    assert(memcmp(value->contents[0].data, expected_text, strlen(expected_text)) == 0);
    assert(value->confirmed == expected_confirmed);
    assert(!value->remember);
    completed++;
}

void ghostty_surface_deny_clipboard_request(ghostty_surface_t surface, void *request) {
    assert(surface == (void *)1 && request == (void *)2);
    denied++;
}

static bool approve(void *userdata, int operation, const char *text) {
    assert(userdata == (void *)3);
    assert(operation == expected_operation);
    assert(strcmp(text, expected_text) == 0);
    asked++;
    return allow;
}

int main(void) {
    const char *plain[] = {"text/plain"};
    const char *image[] = {"image/png"};
    assert(gpui_ghostty_clipboard_wants_text(plain, 1, false));
    assert(!gpui_ghostty_clipboard_wants_text(plain, 1, true));
    assert(!gpui_ghostty_clipboard_wants_text(image, 1, false));
    assert(!gpui_ghostty_clipboard_wants_text(NULL, 0, true));

    // Deliberately unterminated data: the new API promises only these bytes.
    char bytes[] = {'a', 'b', 'c'};
    ghostty_clipboard_content_s content = {.mime = "text/plain", .data = bytes, .len = 3};
    char *text = gpui_ghostty_clipboard_text(&content);
    assert(text != NULL && strcmp(text, "abc") == 0);
    free(text);
    expected_text = "abc";
    gpui_ghostty_complete_text((void *)1, expected_text, (void *)2);
    assert(completed == 1);

    ghostty_clipboard_confirm_s confirm = {.contents = &content, .contents_len = 1};
    gpui_ghostty_confirm_clipboard((void *)1, &confirm, (void *)2,
        GHOSTTY_CLIPBOARD_REQUEST_PASTE, approve, (void *)3);
    assert(denied == 1 && asked == 1 && completed == 1);

    allow = true;
    expected_confirmed = true;
    expected_operation = 1;
    gpui_ghostty_confirm_clipboard((void *)1, &confirm, (void *)2,
        GHOSTTY_CLIPBOARD_REQUEST_OSC_52_READ, approve, (void *)3);
    assert(completed == 2);
    expected_operation = 2;
    gpui_ghostty_confirm_clipboard((void *)1, &confirm, (void *)2,
        GHOSTTY_CLIPBOARD_REQUEST_KITTY_WRITE, approve, (void *)3);
    assert(completed == 3);

    // Never approve only a prefix of data containing an embedded NUL.
    bytes[1] = '\0';
    assert(gpui_ghostty_clipboard_text(&content) == NULL);
    gpui_ghostty_confirm_clipboard((void *)1, &confirm, (void *)2,
        GHOSTTY_CLIPBOARD_REQUEST_KITTY_READ, approve, (void *)3);
    assert(denied == 2 && asked == 3 && completed == 3);

    content.mime = "image/png";
    assert(gpui_ghostty_clipboard_text(&content) == NULL);
    content.mime = "text/plain";
    content.data = NULL;
    content.len = 0;
    text = gpui_ghostty_clipboard_text(&content);
    assert(text != NULL && text[0] == '\0');
    free(text);
    content.len = SIZE_MAX;
    assert(gpui_ghostty_clipboard_text(&content) == NULL);
    return 0;
}
