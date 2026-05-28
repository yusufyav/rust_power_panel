//! Ham Linux syscall katmanı.
//!
//! Zig 0.16 dosya sistemi/I-O API'sini `std.Io`'ya taşıdı ve her çağrı bir
//! `io: Io` örneği ister; bu arayüz henüz kararsız. PowerPanel yalnızca Linux
//! (Wayland) hedeflediği için doğrudan `std.os.linux` syscall'larını kullanmak
//! hem en sağlam hem de en düşük overhead'li yoldur — `std.fs`/`std.Io`
//! sürüm değişimlerinden etkilenmez.

const std = @import("std");
const linux = std.os.linux;

/// Bir syscall dönüş değeri (-errno olarak sarılı) hata mı?
pub fn isErr(rc: usize) bool {
    return @as(isize, @bitCast(rc)) < 0;
}

fn toZ(path: []const u8, buf: *[4096]u8) ?[*:0]const u8 {
    if (path.len >= buf.len) return null;
    @memcpy(buf[0..path.len], path);
    buf[path.len] = 0;
    return @ptrCast(buf);
}

/// stdout'a (fd 1) tüm baytları yaz.
pub fn writeAll(fd: i32, bytes: []const u8) void {
    var off: usize = 0;
    while (off < bytes.len) {
        const rc = linux.write(fd, bytes.ptr + off, bytes.len - off);
        if (isErr(rc)) return;
        if (rc == 0) return;
        off += rc;
    }
}

pub fn stdout(bytes: []const u8) void {
    writeAll(1, bytes);
}

/// Dosyayı tamamen `buf` içine oku; başarısızlıkta null.
pub fn readFile(path: []const u8, buf: []u8) ?[]u8 {
    var pbuf: [4096]u8 = undefined;
    const pz = toZ(path, &pbuf) orelse return null;
    const fd_rc = linux.open(pz, .{ .ACCMODE = .RDONLY, .CLOEXEC = true }, 0);
    if (isErr(fd_rc)) return null;
    const fd: i32 = @intCast(fd_rc);
    defer _ = linux.close(fd);

    var total: usize = 0;
    while (total < buf.len) {
        const rc = linux.read(fd, buf.ptr + total, buf.len - total);
        if (isErr(rc)) return null;
        if (rc == 0) break;
        total += rc;
    }
    return buf[0..total];
}

/// Dosyayı oku, baştaki/sondaki boşlukları kırp.
pub fn readTrim(path: []const u8, buf: []u8) ?[]const u8 {
    const c = readFile(path, buf) orelse return null;
    return std.mem.trim(u8, c, " \t\r\n");
}

/// Dosyadan u64 oku (trim + parse). Rust'taki read_u64 karşılığı.
pub fn readU64(path: []const u8) ?u64 {
    var buf: [64]u8 = undefined;
    const s = readTrim(path, &buf) orelse return null;
    return std.fmt.parseInt(u64, s, 10) catch null;
}

/// Bir yol var mı? (Rust: fs::metadata(..).is_ok())
pub fn exists(path: []const u8) bool {
    var pbuf: [4096]u8 = undefined;
    const pz = toZ(path, &pbuf) orelse return false;
    const rc = linux.open(pz, .{ .ACCMODE = .RDONLY, .PATH = true, .CLOEXEC = true }, 0);
    if (isErr(rc)) return false;
    _ = linux.close(@intCast(rc));
    return true;
}

/// Sembolik bağın hedefinin son bileşenini (basename) döndür.
/// Rust: read_link(...).file_name() — i915/xe sürücü tespiti için.
pub fn readLinkBasename(path: []const u8, buf: []u8) ?[]const u8 {
    var pbuf: [4096]u8 = undefined;
    const pz = toZ(path, &pbuf) orelse return null;
    const rc = linux.readlink(pz, buf.ptr, buf.len);
    if (isErr(rc) or rc == 0) return null;
    return std.fs.path.basename(buf[0..rc]);
}

pub const DT_DIR: u8 = 4;

/// getdents64 tabanlı dizin yineleyici.
pub const DirIter = struct {
    fd: i32,
    buf: [8192]u8 align(8) = undefined,
    buf_len: usize = 0,
    pos: usize = 0,

    pub const Entry = struct {
        name: []const u8,
        kind: u8,
    };

    pub fn open(path: []const u8) ?DirIter {
        var pbuf: [4096]u8 = undefined;
        const pz = toZ(path, &pbuf) orelse return null;
        const rc = linux.open(pz, .{
            .ACCMODE = .RDONLY,
            .DIRECTORY = true,
            .CLOEXEC = true,
        }, 0);
        if (isErr(rc)) return null;
        return .{ .fd = @intCast(rc) };
    }

    pub fn close(self: *DirIter) void {
        _ = linux.close(self.fd);
    }

    pub fn next(self: *DirIter) ?Entry {
        while (true) {
            if (self.pos >= self.buf_len) {
                const rc = linux.getdents64(self.fd, &self.buf, self.buf.len);
                if (isErr(rc) or rc == 0) return null;
                self.buf_len = rc;
                self.pos = 0;
            }
            const d: *align(1) linux.dirent64 = @ptrCast(&self.buf[self.pos]);
            const reclen: usize = d.reclen;
            // name, dirent64 başlangıcından 19 bayt sonra başlar (ino8+off8+reclen2+type1)
            const name_ptr: [*:0]const u8 = @ptrCast(&self.buf[self.pos + 19]);
            const name = std.mem.span(name_ptr);
            const kind = d.type;
            self.pos += reclen;
            if (std.mem.eql(u8, name, ".") or std.mem.eql(u8, name, "..")) continue;
            return .{ .name = name, .kind = kind };
        }
    }
};

/// Monotonik saat — nanosaniye.
pub fn nowNs() u64 {
    var ts: linux.timespec = undefined;
    _ = linux.clock_gettime(.MONOTONIC, &ts);
    return @as(u64, @intCast(ts.sec)) * 1_000_000_000 + @as(u64, @intCast(ts.nsec));
}

pub fn sleepMs(ms: u64) void {
    var req = linux.timespec{
        .sec = @intCast(ms / 1000),
        .nsec = @intCast((ms % 1000) * 1_000_000),
    };
    // EINTR durumunda kalan süreyle devam et
    while (linux.nanosleep(&req, &req) != 0) {}
}
