#ifndef GPUI_GHOSTTY_CLIPBOARD_H
#define GPUI_GHOSTTY_CLIPBOARD_H

#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <ghostty.h>

// The GPUI adapter exposes text only. Do not read data for MIME-list requests
// or claim support for binary representations that the host cannot deliver.
static inline bool gpui_ghostty_clipboard_wants_text(
    const char *const *mimes, size_t count, bool list
) {
    return !list && count == 1 && mimes != NULL && mimes[0] != NULL &&
        strcmp(mimes[0], "text/plain") == 0;
}

// Rust approval callbacks and the host clipboard API take C strings. Reject
// embedded NULs so approval always sees the entire value we would transfer.
static inline char *gpui_ghostty_clipboard_text(const ghostty_clipboard_content_s *content) {
    if (content == NULL || content->mime == NULL ||
        strcmp(content->mime, "text/plain") != 0 || content->len == SIZE_MAX ||
        (content->len != 0 && (content->data == NULL ||
            memchr(content->data, '\0', content->len) != NULL))) return NULL;
    char *text = malloc(content->len + 1);
    if (text == NULL) return NULL;
    if (content->len != 0) memcpy(text, content->data, content->len);
    text[content->len] = '\0';
    return text;
}

static inline void gpui_ghostty_complete_text(
    ghostty_surface_t surface, const char *text, void *request
) {
    const ghostty_clipboard_content_s content = {
        .mime = "text/plain", .data = text, .len = strlen(text),
    };
    const ghostty_clipboard_complete_s complete = {
        .contents = &content, .contents_len = 1,
    };
    ghostty_surface_complete_clipboard_request(surface, &complete, request);
}

static inline void gpui_ghostty_confirm_clipboard(
    ghostty_surface_t surface,
    const ghostty_clipboard_confirm_s *confirm,
    void *request,
    ghostty_clipboard_request_e kind,
    bool (*approve)(void *, int, const char *),
    void *userdata
) {
    char *text = confirm->contents_len == 1
        ? gpui_ghostty_clipboard_text(&confirm->contents[0]) : NULL;
    int operation = kind == GHOSTTY_CLIPBOARD_REQUEST_PASTE ? 0 :
        (kind == GHOSTTY_CLIPBOARD_REQUEST_KITTY_WRITE ||
         kind == GHOSTTY_CLIPBOARD_REQUEST_OSC_52_WRITE) ? 2 : 1;
    bool approved = text != NULL && approve != NULL && approve(userdata, operation, text);
    free(text);
    if (!approved) {
        ghostty_surface_deny_clipboard_request(surface, request);
        return;
    }
    const ghostty_clipboard_complete_s complete = {
        .contents = confirm->contents,
        .contents_len = confirm->contents_len,
        .available = confirm->available,
        .available_len = confirm->available_len,
        .confirmed = true,
        .remember = false,
    };
    ghostty_surface_complete_clipboard_request(surface, &complete, request);
}

#endif
