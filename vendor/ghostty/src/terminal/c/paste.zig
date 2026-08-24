const std = @import("std");
const lib = @import("../lib.zig");
const paste = @import("../../input/paste.zig");
const terminal_paste_pkg = @import("../paste.zig");
const clipboard = @import("../clipboard.zig");
const terminal_c = @import("terminal.zig");
const Terminal = terminal_c.Terminal;
const ClipboardContent = terminal_c.ClipboardContent;
const ClipboardRead = terminal_c.ClipboardRead;
const ClipboardReadReply = terminal_c.ClipboardReadReply;
const Result = @import("result.zig").Result;

/// Why a paste happened.
///
/// C: GhosttyPasteSource
pub const Source = terminal_paste_pkg.Source;

/// A paste of clipboard contents into the terminal. Sized struct.
///
/// C: GhosttyPaste
pub const Request = extern struct {
    size: usize = @sizeOf(Request),
    location: clipboard.Location,
    source: Source,
    contents: ?[*]const ClipboardContent,
    contents_len: usize,
    allow_unsafe: bool,
};

pub fn terminal_paste(
    terminal_: Terminal,
    req_: ?*const Request,
    out_written: ?*bool,
) callconv(lib.calling_conv) Result {
    const wrapper = terminal_ orelse return .invalid_value;
    const req = req_ orelse return .invalid_value;

    // Every field is required; a smaller size is a caller from a
    // different ABI version than any this struct has had.
    if (req.size < @sizeOf(Request)) return .invalid_value;

    // The handler always has a write_pty trampoline that no-ops without
    // a C callback, so the "nothing can be written" check is ours.
    if (wrapper.effects.write_pty == null) return .invalid_value;

    const c_contents: []const ClipboardContent = if (req.contents) |ptr|
        ptr[0..req.contents_len]
    else
        &.{};

    // A paste carries a handful of representations, so keep the common
    // case allocation-free.
    var sfa = std.heap.stackFallback(256, wrapper.terminal.gpa());
    const alloc = sfa.get();
    const contents = alloc.alloc(
        clipboard.Content,
        c_contents.len,
    ) catch return .out_of_memory;
    defer alloc.free(contents);
    for (contents, c_contents) |*content, c_content| {
        content.* = .{
            .mime = c_content.mime.ptr[0..c_content.mime.len],
            .data = c_content.data.ptr[0..c_content.data.len],
        };
    }

    const written = wrapper.stream.handler.paste(.{
        .location = req.location,
        .source = req.source,
        .contents = contents,
        .allow_unsafe = req.allow_unsafe,
    }) catch |err| return switch (err) {
        error.UnsafePaste => .rejected,
        error.NoWritePty => .invalid_value,
        error.OutOfMemory => .out_of_memory,
        error.EntropyUnavailable, error.Canceled => .io_error,
    };
    if (out_written) |ptr| ptr.* = written;
    return .success;
}

pub fn is_safe(data: ?[*]const u8, len: usize) callconv(lib.calling_conv) bool {
    const slice: []const u8 = if (data) |v| v[0..len] else &.{};
    return paste.isSafe(slice);
}

pub fn encode(
    data: ?[*]u8,
    data_len: usize,
    bracketed: bool,
    out_: ?[*]u8,
    out_len: usize,
    out_written: *usize,
) callconv(lib.calling_conv) Result {
    const slice: []u8 = if (data) |v| v[0..data_len] else &.{};
    const result = paste.encode(slice, .{ .bracketed = bracketed });

    const total = result[0].len + result[1].len + result[2].len;
    out_written.* = total;

    const out: []u8 = if (out_) |o| o[0..out_len] else &.{};
    if (out.len < total) return .out_of_space;

    var offset: usize = 0;
    for (result) |segment| {
        @memcpy(out[offset..][0..segment.len], segment);
        offset += segment.len;
    }

    return .success;
}

test "encode bracketed" {
    const testing = std.testing;
    const input = try testing.allocator.dupe(u8, "hello");
    defer testing.allocator.free(input);
    var buf: [64]u8 = undefined;
    var written: usize = 0;
    const result = encode(input.ptr, input.len, true, &buf, buf.len, &written);
    try testing.expectEqual(.success, result);
    try testing.expectEqualStrings("\x1b[200~hello\x1b[201~", buf[0..written]);
}

test "encode unbracketed no newlines" {
    const testing = std.testing;
    const input = try testing.allocator.dupe(u8, "hello");
    defer testing.allocator.free(input);
    var buf: [64]u8 = undefined;
    var written: usize = 0;
    const result = encode(input.ptr, input.len, false, &buf, buf.len, &written);
    try testing.expectEqual(.success, result);
    try testing.expectEqualStrings("hello", buf[0..written]);
}

test "encode unbracketed newlines" {
    const testing = std.testing;
    const input = try testing.allocator.dupe(u8, "hello\nworld");
    defer testing.allocator.free(input);
    var buf: [64]u8 = undefined;
    var written: usize = 0;
    const result = encode(input.ptr, input.len, false, &buf, buf.len, &written);
    try testing.expectEqual(.success, result);
    try testing.expectEqualStrings("hello\rworld", buf[0..written]);
}

test "encode strip unsafe bytes" {
    const testing = std.testing;
    const input = try testing.allocator.dupe(u8, "hel\x1blo\x00world");
    defer testing.allocator.free(input);
    var buf: [64]u8 = undefined;
    var written: usize = 0;
    const result = encode(input.ptr, input.len, true, &buf, buf.len, &written);
    try testing.expectEqual(.success, result);
    try testing.expectEqualStrings("\x1b[200~hel lo world\x1b[201~", buf[0..written]);
}

test "encode with insufficient buffer" {
    const testing = std.testing;
    const input = try testing.allocator.dupe(u8, "hello");
    defer testing.allocator.free(input);
    var buf: [1]u8 = undefined;
    var written: usize = 0;
    const result = encode(input.ptr, input.len, true, &buf, buf.len, &written);
    try testing.expectEqual(.out_of_space, result);
    try testing.expectEqual(17, written);
}

test "encode with null buffer" {
    const testing = std.testing;
    const input = try testing.allocator.dupe(u8, "hello");
    defer testing.allocator.free(input);
    var written: usize = 0;
    const result = encode(input.ptr, input.len, true, null, 0, &written);
    try testing.expectEqual(.out_of_space, result);
    try testing.expectEqual(17, written);
}

test "is_safe with safe data" {
    const testing = std.testing;
    const safe = "hello world";
    try testing.expect(is_safe(safe.ptr, safe.len));
}

test "is_safe with newline" {
    const testing = std.testing;
    const unsafe = "hello\nworld";
    try testing.expect(!is_safe(unsafe.ptr, unsafe.len));
}

test "is_safe with bracketed paste end" {
    const testing = std.testing;
    const unsafe = "hello\x1b[201~world";
    try testing.expect(!is_safe(unsafe.ptr, unsafe.len));
}

test "is_safe with empty data" {
    const testing = std.testing;
    const empty = "";
    try testing.expect(is_safe(empty.ptr, 0));
}

test "is_safe with null empty data" {
    const testing = std.testing;
    try testing.expect(is_safe(null, 0));
}

/// Capture state for the terminal_paste tests: every pty write and the
/// clipboard reads that follow a paste event.
const TerminalPasteCapture = struct {
    var written: [1024]u8 = undefined;
    var written_len: usize = 0;
    var write_count: usize = 0;
    var read_count: usize = 0;
    var last_read_granted: bool = false;

    fn reset() void {
        written_len = 0;
        write_count = 0;
        read_count = 0;
        last_read_granted = false;
    }

    fn writePty(_: Terminal, _: ?*anyopaque, ptr: [*]const u8, len: usize) callconv(lib.calling_conv) void {
        @memcpy(written[written_len..][0..len], ptr[0..len]);
        written_len += len;
        write_count += 1;
    }

    fn clipboardRead(_: Terminal, _: ?*anyopaque, request: *const ClipboardRead) callconv(lib.calling_conv) void {
        read_count += 1;
        last_read_granted = request.granted;
        const contents = [_]ClipboardContent{.{
            .mime = .init(@as([]const u8, "text/plain")),
            .data = .init(@as([]const u8, "Ghostty")),
        }};
        request.reply(request, &.{
            .size = @sizeOf(ClipboardReadReply),
            .result = .success,
            .contents = &contents,
            .contents_len = contents.len,
            .available = null,
            .available_len = 0,
            .remember = false,
        });
    }

    fn writtenSlice() []const u8 {
        return written[0..written_len];
    }

    /// A single text/plain request; the caller provides the content
    /// storage since the request borrows it.
    fn textRequest(content: *ClipboardContent, text: []const u8) Request {
        content.* = .{
            .mime = .init(@as([]const u8, "text/plain")),
            .data = .init(text),
        };
        return .{
            .location = .standard,
            .source = .clipboard,
            .contents = content[0..1],
            .contents_len = 1,
            .allow_unsafe = false,
        };
    }
};

test "terminal_paste null handling" {
    const testing = std.testing;
    const S = TerminalPasteCapture;

    var t: Terminal = null;
    try testing.expectEqual(Result.success, terminal_c.new(&lib.alloc.test_allocator, &t, 80, 24));
    defer terminal_c.free(t);

    var written: bool = true;
    var content: ClipboardContent = undefined;
    const req: Request = S.textRequest(&content, "hello");
    try testing.expectEqual(Result.invalid_value, terminal_paste(null, &req, &written));
    try testing.expectEqual(Result.invalid_value, terminal_paste(t, null, &written));
    try testing.expect(written);

    // A size smaller than the struct is rejected.
    var small = req;
    small.size = @sizeOf(usize);
    try testing.expectEqual(Result.invalid_value, terminal_paste(t, &small, &written));
}

test "terminal_paste without write_pty is invalid" {
    const testing = std.testing;
    const S = TerminalPasteCapture;
    S.reset();

    var t: Terminal = null;
    try testing.expectEqual(Result.success, terminal_c.new(&lib.alloc.test_allocator, &t, 80, 24));
    defer terminal_c.free(t);

    var content: ClipboardContent = undefined;
    const req: Request = S.textRequest(&content, "hello");
    try testing.expectEqual(Result.invalid_value, terminal_paste(t, &req, null));
}

test "terminal_paste text and unsafe" {
    const testing = std.testing;
    const S = TerminalPasteCapture;
    S.reset();

    var t: Terminal = null;
    try testing.expectEqual(Result.success, terminal_c.new(&lib.alloc.test_allocator, &t, 80, 24));
    defer terminal_c.free(t);
    try testing.expectEqual(Result.success, terminal_c.set(t, .write_pty, @ptrCast(&S.writePty)));

    // Plain text, NULL out_written pointer is fine.
    var content: ClipboardContent = undefined;
    const req: Request = S.textRequest(&content, "hel\x1blo");
    try testing.expectEqual(Result.success, terminal_paste(t, &req, null));
    try testing.expectEqualStrings("hel lo", S.writtenSlice());
    try testing.expectEqual(@as(usize, 1), S.write_count);

    // Unsafe is refused with nothing written, then allowed.
    S.reset();
    var written: bool = false;
    var unsafe_content: ClipboardContent = undefined;
    var unsafe: Request = S.textRequest(&unsafe_content, "rm -rf /\n");
    try testing.expectEqual(Result.rejected, terminal_paste(t, &unsafe, &written));
    try testing.expectEqual(@as(usize, 0), S.write_count);
    try testing.expect(!written);

    unsafe.allow_unsafe = true;
    try testing.expectEqual(Result.success, terminal_paste(t, &unsafe, &written));
    try testing.expect(written);
    try testing.expectEqualStrings("rm -rf /\r", S.writtenSlice());

    // Bracketed paste mode frames the text through the real mode path.
    S.reset();
    const decset = "\x1b[?2004h";
    terminal_c.vt_write(t, decset, decset.len);
    try testing.expectEqual(Result.success, terminal_paste(t, &req, &written));
    try testing.expect(written);
    try testing.expectEqualStrings("\x1b[200~hel lo\x1b[201~", S.writtenSlice());

    // No text representation writes nothing. NULL contents with a zero
    // length is an empty list.
    S.reset();
    var empty_content: ClipboardContent = undefined;
    var empty: Request = S.textRequest(&empty_content, "");
    empty.contents = null;
    empty.contents_len = 0;
    try testing.expectEqual(Result.success, terminal_paste(t, &empty, &written));
    try testing.expect(!written);
    try testing.expectEqual(@as(usize, 0), S.write_count);
}

test "terminal_paste event" {
    const testing = std.testing;
    const S = TerminalPasteCapture;
    S.reset();

    var t: Terminal = null;
    try testing.expectEqual(Result.success, terminal_c.new(&lib.alloc.test_allocator, &t, 80, 24));
    defer terminal_c.free(t);
    try testing.expectEqual(Result.success, terminal_c.set(t, .write_pty, @ptrCast(&S.writePty)));
    t.?.terminal.modes.set(.kitty_paste_events, true);

    // Without a clipboard_read callback the paste stays text.
    var written: bool = false;
    const contents = [_]ClipboardContent{
        .{
            .mime = .init(@as([]const u8, "text/plain")),
            .data = .init(@as([]const u8, "secret")),
        },
        .{
            .mime = .init(@as([]const u8, "image/png")),
            .data = .init(@as([]const u8, "")),
        },
    };
    const req: Request = .{
        .location = .primary,
        .source = .clipboard,
        .contents = &contents,
        .contents_len = contents.len,
        .allow_unsafe = false,
    };
    try testing.expectEqual(Result.success, terminal_paste(t, &req, &written));
    try testing.expect(written);
    try testing.expectEqualStrings("secret", S.writtenSlice());

    // With one, an event is sent listing every MIME type and the data
    // is never written.
    S.reset();
    try testing.expectEqual(Result.success, terminal_c.set(t, .clipboard_read, @ptrCast(&S.clipboardRead)));
    try testing.expectEqual(Result.success, terminal_paste(t, &req, &written));
    try testing.expect(written);
    try testing.expectEqual(@as(usize, 1), S.write_count);
    try testing.expectEqual(@as(usize, 3), std.mem.count(u8, S.writtenSlice(), "\x1b]5522;"));
    try testing.expect(std.mem.startsWith(u8, S.writtenSlice(), "\x1b]5522;type=read:status=OK:loc=primary:pw="));
    try testing.expect(std.mem.indexOf(u8, S.writtenSlice(), "secret") == null);
    try testing.expect(std.mem.indexOf(u8, S.writtenSlice(), ";dGV4dC9wbGFpbiBpbWFnZS9wbmcK\x1b\\") != null);

    // The program's read with the event password is granted once.
    const ok_prefix = "\x1b]5522;type=read:status=OK:loc=primary:pw=";
    const pw_end = std.mem.indexOfPos(u8, S.writtenSlice(), ok_prefix.len, "\x1b\\").?;
    var read_buf: [256]u8 = undefined;
    const read = try std.fmt.bufPrint(
        &read_buf,
        "\x1b]5522;type=read:pw={s}:name=UGFzdGUgZXZlbnQ=;dGV4dC9wbGFpbg==\x1b\\",
        .{S.writtenSlice()[ok_prefix.len..pw_end]},
    );
    S.reset();
    terminal_c.vt_write(t, read.ptr, read.len);
    try testing.expectEqual(@as(usize, 1), S.read_count);
    try testing.expect(S.last_read_granted);
    try testing.expect(std.mem.indexOf(u8, S.writtenSlice(), ";R2hvc3R0eQ==\x1b\\") != null);

    S.reset();
    terminal_c.vt_write(t, read.ptr, read.len);
    try testing.expectEqual(@as(usize, 1), S.read_count);
    try testing.expect(!S.last_read_granted);

    // Text sources never become events.
    S.reset();
    var ime = req;
    ime.source = .text;
    try testing.expectEqual(Result.success, terminal_paste(t, &ime, &written));
    try testing.expect(written);
    try testing.expectEqualStrings("secret", S.writtenSlice());
}
