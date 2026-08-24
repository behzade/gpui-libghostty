//! Pasting into a terminal.
//!
//! This is the single place that turns "the user pasted" into bytes for
//! the pty, applying the terminal's current state:
//!
//!   * Mode 5522 (Kitty clipboard protocol paste events) set, a
//!     user-initiated clipboard paste, and the embedder able to serve
//!     the program's follow-up clipboard read: send a paste event
//!     listing the clipboard's MIME types with a fresh one-time password
//!     and record a one-time read grant for it. The data is not written.
//!   * Otherwise: write the first text representation, with unsafe bytes
//!     replaced (xterm behavior), framed per mode 2004 (bracketed paste)
//!     or with newlines converted to carriage returns if not.
//!
//! The precedence (5522 event, else 2004 framing, else plain) and the
//! safety rule live only here so every embedder of the terminal shares
//! one implementation.

const std = @import("std");
const Allocator = std.mem.Allocator;
const lib = @import("lib.zig");
const clipboard = @import("clipboard.zig");
const kitty_clipboard = @import("kitty/clipboard.zig");
const input_paste = @import("../input/paste.zig");
const Terminal = @import("Terminal.zig");

/// Why a paste happened. Only clipboard pastes may become paste events.
///
/// C: GhosttyPasteSource
pub const Source = lib.Enum(lib.target, &.{
    // The user pasted from a clipboard: keybind, menu, middle click.
    "clipboard",

    // Text inserted some other way: IME commit, drag and drop,
    // scripted input. Never becomes a paste event, matching kitty.
    // This is not a way to opt out of events; an embedder that
    // doesn't want them doesn't serve clipboard reads.
    "text",
});

/// A paste of clipboard contents into the terminal. What actually gets
/// written depends on terminal state; see `paste`.
pub const Request = struct {
    /// The clipboard the contents came from. Reported to the program on
    /// a paste event (`.primary` and `.selection` both as loc=primary,
    /// the protocol knows only two); no effect on a text paste.
    location: clipboard.Location = .standard,

    /// Why this paste happened. Only a user-initiated clipboard paste
    /// may become a paste event; text insertion always writes text.
    source: Source = .clipboard,

    /// The representations available, in the embedder's preferred
    /// order. A text paste writes the first representation with a text
    /// MIME type (clipboard.isTextMime) and ignores the rest. A paste
    /// event reports every MIME type and never touches data, so
    /// non-text entries may carry empty data. Borrowed for the call.
    contents: []const clipboard.Content,

    /// Write data that could inject commands (see `isSafe`). The usual
    /// flow is to call with false, confirm with the user on
    /// error.UnsafePaste, and call again with true.
    allow_unsafe: bool = false,
};

/// What a caller supplies to `paste`: the terminal state the decision
/// depends on, the session state an event records into, and the sink.
pub const Context = struct {
    /// The terminal whose modes decide the encoding.
    terminal: *const Terminal,

    /// Kitty clipboard session grants. A paste event records its
    /// one-time password here so the program's follow-up read is
    /// served without a prompt.
    grants: *kitty_clipboard.Grants,

    /// Secure entropy for one-time passwords. See generateOtp for why
    /// there is no fallback when this has none.
    io: std.Io,

    /// Allocator for the grant. Must be the one `grants` is freed with.
    alloc: Allocator,

    /// True if the embedder serves clipboard reads, so a paste event's
    /// follow-up read can be answered. Without that an event would be
    /// refused and the user's paste would vanish, so `paste` falls
    /// through to a text paste instead.
    can_event: bool,

    /// Receives the bytes for the pty. `paste` makes exactly one logical
    /// write per call: the whole encoded text or the whole event. On
    /// error the writer may hold a partial result that must be
    /// discarded.
    writer: *std.Io.Writer,
};

pub const Error = Allocator.Error || std.Io.RandomSecureError || std.Io.Writer.Error || error{
    /// The data could inject commands and allow_unsafe was false.
    /// Nothing was written.
    UnsafePaste,
};

/// Paste into the terminal, applying the terminal's current state as
/// described in the module docs. Returns true if anything was written
/// to `ctx.writer`: the encoded text or a paste event. False means
/// there was nothing to paste (no non-empty text representation).
///
/// The safety rule for a text paste (`input.paste.isSafeWith`): a
/// bracketed paste is unsafe only if it contains the bracket terminator
/// (CSI 201~); an unbracketed paste is unsafe if it contains a newline
/// or the terminator. Embedders wanting a stricter rule check
/// `input.paste.isSafe` themselves before calling. A paste event never
/// puts the data on the input stream, so the rule doesn't apply to it.
///
/// On success, the caller delivers the writer's contents to the pty.
/// On error nothing should be delivered; in particular an event's grant
/// is only recorded once the event is fully encoded, so a failure never
/// leaves a grant for an event that was never sent.
pub fn paste(ctx: Context, req: Request) Error!bool {
    // A paste event only works if the program's follow-up read can be
    // served; without that, fall through to text.
    if (req.source == .clipboard and
        ctx.can_event and
        ctx.terminal.modes.get(.kitty_paste_events))
    {
        try pasteKittyEvent(ctx, req);
        return true;
    }

    // For non-Kitty paste events we can only accept text content.
    const text: []const u8 = for (req.contents) |c| {
        if (clipboard.isTextMime(c.mime)) break c.data;
    } else return false;
    if (text.len == 0) return false;

    // Reject unsafe inputs
    const opts: input_paste.Options = .fromTerminal(ctx.terminal);
    if (!req.allow_unsafe and !input_paste.isSafeWith(text, opts)) {
        return error.UnsafePaste;
    }

    // The data is copied exactly once, into the writer, where the
    // encoder strips and converts it in place.
    try input_paste.encodeWriter(ctx.writer, text, opts);
    return true;
}

fn pasteKittyEvent(ctx: Context, req: Request) Error!void {
    const otp = try kitty_clipboard.generateOtp(ctx.io);

    // Every representation is listed, never read. The listing is
    // bounded; a clipboard with more types than that is not a thing.
    var mimes_buf: [kitty_clipboard.max_listing_mimes][]const u8 = undefined;
    var mimes_len: usize = 0;
    for (req.contents) |c| {
        if (mimes_len == mimes_buf.len) break;
        mimes_buf[mimes_len] = c.mime;
        mimes_len += 1;
    }

    try (kitty_clipboard.PasteEvent{
        // The protocol only distinguishes the clipboard from the
        // primary selection, so both non-standard locations report as
        // primary.
        .primary = req.location != .standard,
        .pw = &otp,
        .available = mimes_buf[0..mimes_len],
    }).encode(ctx.writer);

    // Recorded last so a failed encode leaves no grant behind. The
    // caller delivers the event after we return, so the grant is in
    // place before the program can possibly use it.
    try ctx.grants.grant(
        ctx.alloc,
        &otp,
        .read,
        true,
    );
}

test {
    // The behavior is tested end to end through the stream handler
    // (stream_terminal.zig), which is the primary caller.
    std.testing.refAllDecls(@This());
}
