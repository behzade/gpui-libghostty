/**
 * @file paste.h
 *
 * Paste - paste into a terminal, and validate and encode paste data.
 */

#ifndef GHOSTTY_VT_PASTE_H
#define GHOSTTY_VT_PASTE_H

/** @defgroup paste Paste
 *
 * Pasting into a terminal, plus the terminal-free utilities for
 * validating and encoding paste data.
 *
 * ## Pasting into a Terminal
 *
 * What a paste writes to the pty depends on the terminal's state, so
 * the recommended way to paste is ghostty_terminal_paste(). The embedder
 * hands over what the clipboard holds as MIME-typed contents (just
 * `text/plain` for an ordinary paste) and where it came from, and the
 * terminal decides how its current modes apply:
 *
 * - If Kitty clipboard protocol paste events (mode 5522,
 *   GHOSTTY_MODE_PASTE_EVENTS) are enabled, the paste was user-initiated
 *   (GHOSTTY_PASTE_SOURCE_CLIPBOARD), and a clipboard_read callback is
 *   installed, the terminal sends the program a paste event listing the
 *   clipboard's MIME types with a one-time password instead of the data.
 *   The program then reads what it wants through the clipboard_read
 *   callback, which arrives with `granted` set so no permission prompt
 *   is needed.
 * - Otherwise the first text representation is written: unsafe control
 *   bytes are replaced with spaces, and it is wrapped in bracketed paste
 *   sequences if mode 2004 (GHOSTTY_MODE_BRACKETED_PASTE) is enabled, or
 *   has its newlines converted to carriage returns if not.
 *
 * Text that could inject commands (a newline when unbracketed, or the
 * bracketed paste terminator when bracketed) is refused with
 * GHOSTTY_REJECTED unless GhosttyPaste::allow_unsafe is set. The usual
 * flow is to call once, confirm with the user on GHOSTTY_REJECTED, and
 * call again with `allow_unsafe` set.
 *
 * Output is delivered through the write_pty callback
 * (GHOSTTY_TERMINAL_OPT_WRITE_PTY) in a single call.
 *
 * @snippet c-vt-paste/src/main.c terminal-paste
 *
 * ## Building Blocks
 *
 * For embedders that encode without a terminal, ghostty_paste_is_safe()
 * checks if paste data contains potentially dangerous sequences
 * (conservatively, regardless of terminal state) and
 * ghostty_paste_encode() encodes paste data for writing to the pty,
 * including bracketed paste wrapping and unsafe byte stripping.
 *
 * ### Safety Check
 *
 * @snippet c-vt-paste/src/main.c paste-safety
 *
 * ### Encoding
 *
 * @snippet c-vt-paste/src/main.c paste-encode
 *
 * @{
 */

#include <stdbool.h>
#include <stddef.h>
#include <ghostty/vt/types.h>
#include <ghostty/vt/terminal.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Why a paste happened. 
 */
typedef enum GHOSTTY_ENUM_TYPED {
  /** The user pasted from a clipboard: keybind, menu, middle click. */
  GHOSTTY_PASTE_SOURCE_CLIPBOARD = 0,

  /**
   * Text inserted some other way: IME commit, drag and drop, scripted
   * input. Always written as text, never as a paste event, matching
   * kitty. This is not a way to opt out of paste events; an embedder
   * that doesn't want them doesn't install a clipboard_read callback.
   */
  GHOSTTY_PASTE_SOURCE_TEXT = 1,
  GHOSTTY_PASTE_SOURCE_MAX_VALUE = GHOSTTY_ENUM_MAX_VALUE,
} GhosttyPasteSource;

/**
 * A paste of clipboard contents into the terminal.
 *
 * This is a sized struct; set `size` to `sizeof(GhosttyPaste)`. The
 * contents array and the strings it points to are borrowed only for the
 * duration of the ghostty_terminal_paste() call.
 */
typedef struct {
  /** Size of this struct in bytes. */
  size_t size;

  /**
   * The clipboard the contents came from. Reported to the program on a
   * paste event (the selection and primary locations are both reported
   * as the primary selection, the protocol knows only two); no effect
   * on a text paste.
   */
  GhosttyClipboardLocation location;

  /** Why this paste happened. */
  GhosttyPasteSource source;

  /**
   * Borrowed array of the representations available, in preferred
   * order. A text paste writes the first entry with a text MIME type
   * such as "text/plain" and ignores the rest. A paste event lists every
   * entry's MIME type and never reads data, so non-text entries may have
   * empty data. May be NULL when contents_len is zero.
   */
  const GhosttyClipboardContent* contents;

  /** Number of entries in contents. */
  size_t contents_len;

  /**
   * Write text that could inject commands. Call with false, confirm
   * with the user on GHOSTTY_REJECTED, and call again with true.
   */
  bool allow_unsafe;
} GhosttyPaste;

/**
 * Paste into the terminal according to its current state: a Kitty
 * clipboard protocol paste event if mode 5522 is enabled and a
 * clipboard_read callback is installed, otherwise the text framed per
 * mode 2004. See the group documentation for the full behavior. Output
 * goes through the write_pty callback in a single call. The viewport is
 * not scrolled; that is up to the embedder, as for key input.
 *
 * @param terminal The terminal handle
 * @param paste The paste request, borrowed for the duration of the call
 * @param[out] out_written On success, whether anything was written to
 *             the pty (the encoded text or a paste event). False means
 *             there was nothing to paste: no non-empty text
 *             representation. May be NULL.
 * @return GHOSTTY_SUCCESS on success (see @p out_written);
 *         GHOSTTY_REJECTED if the text could inject commands and
 *         GhosttyPaste::allow_unsafe is false (nothing was written);
 *         GHOSTTY_INVALID_VALUE for a NULL terminal or paste, or when no
 *         write_pty callback is installed; GHOSTTY_OUT_OF_MEMORY;
 *         GHOSTTY_IO_ERROR if there is no secure entropy source to mint
 *         a paste event password (wasm32-freestanding without
 *         GHOSTTY_SYS_OPT_RANDOM_SECURE set), in which case nothing was
 *         written and no grant was recorded.
 */
GHOSTTY_API GhosttyResult ghostty_terminal_paste(
    GhosttyTerminal terminal,
    const GhosttyPaste* paste,
    bool* out_written);

/**
 * Check if paste data is safe to paste into the terminal.
 *
 * Data is considered unsafe if it contains:
 * - Newlines (`\n`) which can inject commands
 * - The bracketed paste end sequence (`\x1b[201~`) which can be used
 *   to exit bracketed paste mode and inject commands
 *
 * This check is conservative and considers data unsafe regardless of
 * current terminal state. ghostty_terminal_paste() applies the
 * terminal-state-aware rule itself (newlines are safe inside a
 * bracketed paste); use this to apply the stricter rule on top.
 *
 * @param data The paste data to check (must not be NULL)
 * @param len The length of the data in bytes
 * @return true if the data is safe to paste, false otherwise
 */
GHOSTTY_API bool ghostty_paste_is_safe(const char* data, size_t len);

/**
 * Encode paste data for writing to the terminal pty.
 *
 * This function prepares paste data for terminal input by:
 * - Stripping unsafe control bytes (NUL, ESC, DEL, etc.) by replacing
 *   them with spaces
 * - Wrapping the data in bracketed paste sequences if @p bracketed is true
 * - Replacing newlines with carriage returns if @p bracketed is false
 *
 * The input @p data buffer is modified in place during encoding. The
 * encoded result (potentially with bracketed paste prefix/suffix) is
 * written to the output buffer.
 *
 * If the output buffer is too small, the function returns
 * GHOSTTY_OUT_OF_SPACE and sets the required size in @p out_written.
 * The caller can then retry with a sufficiently sized buffer.
 *
 * This is the encoder ghostty_terminal_paste() uses for a text paste;
 * use it directly when there is no terminal to paste into.
 *
 * @param data The paste data to encode (modified in place, may be NULL)
 * @param data_len The length of the input data in bytes
 * @param bracketed Whether bracketed paste mode is active
 * @param buf Output buffer to write the encoded result into (may be NULL)
 * @param buf_len Size of the output buffer in bytes
 * @param[out] out_written On success, the number of bytes written. On
 *             GHOSTTY_OUT_OF_SPACE, the required buffer size.
 * @return GHOSTTY_SUCCESS on success, GHOSTTY_OUT_OF_SPACE if the buffer
 *         is too small
 */
GHOSTTY_API GhosttyResult ghostty_paste_encode(
    char* data,
    size_t data_len,
    bool bracketed,
    char* buf,
    size_t buf_len,
    size_t* out_written);

#ifdef __cplusplus
}
#endif

/** @} */

#endif /* GHOSTTY_VT_PASTE_H */
