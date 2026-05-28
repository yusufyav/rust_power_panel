//! NVML (NVIDIA Management Library) için minimal runtime sarmalayıcı.
//!
//! Rust tarafı `nvml-wrapper` kullanır; o da libnvidia-ml'i çalışma zamanında
//! `dlopen` eder. Burada aynısını yaparız: link zamanında bağımlılık yok, yani
//! NVIDIA sürücüsü olmayan (AMD/Intel) makinelerde de binary sorunsuz çalışır.
//!
//! Struct layout'ları /opt/cuda/include/nvml.h'ten birebir alındı.

const std = @import("std");

pub const Device = ?*anyopaque; // nvmlDevice_t — opak işaretçi

pub const Memory = extern struct {
    total: c_ulonglong,
    free: c_ulonglong,
    used: c_ulonglong,
};

pub const Utilization = extern struct {
    gpu: c_uint,
    memory: c_uint,
};

// nvmlProcessUtilizationSample_t
pub const ProcUtilSample = extern struct {
    pid: c_uint,
    timeStamp: c_ulonglong,
    smUtil: c_uint,
    memUtil: c_uint,
    encUtil: c_uint,
    decUtil: c_uint,
};

// nvmlProcessInfo_t (= nvmlProcessInfo_v2_t)
pub const ProcessInfo = extern struct {
    pid: c_uint,
    usedGpuMemory: c_ulonglong,
    gpuInstanceId: c_uint,
    computeInstanceId: c_uint,
};

const NVML_TEMPERATURE_GPU: c_uint = 0;
const NVML_SUCCESS: c_int = 0;

const InitFn = *const fn () callconv(.c) c_int;
const ShutdownFn = *const fn () callconv(.c) c_int;
const HandleFn = *const fn (c_uint, *Device) callconv(.c) c_int;
const PowerFn = *const fn (Device, *c_uint) callconv(.c) c_int;
const TempFn = *const fn (Device, c_uint, *c_uint) callconv(.c) c_int;
const MemFn = *const fn (Device, *Memory) callconv(.c) c_int;
const UtilFn = *const fn (Device, *Utilization) callconv(.c) c_int;
const ComputeProcFn = *const fn (Device, *c_uint, ?[*]ProcessInfo) callconv(.c) c_int;
const ProcUtilFn = *const fn (Device, ?[*]ProcUtilSample, *c_uint, c_ulonglong) callconv(.c) c_int;

pub const Nvml = struct {
    lib: std.DynLib,
    device: Device,
    getPower: PowerFn,
    getTemp: TempFn,
    getMem: MemFn,
    getUtil: UtilFn,
    getComputeProcs: ComputeProcFn,
    getProcUtil: ProcUtilFn,

    pub fn init() ?Nvml {
        var lib = std.DynLib.open("libnvidia-ml.so.1") catch
            (std.DynLib.open("libnvidia-ml.so") catch return null);

        const initFn = lib.lookup(InitFn, "nvmlInit_v2") orelse return null;
        if (initFn() != NVML_SUCCESS) return null;

        const handleFn = lib.lookup(HandleFn, "nvmlDeviceGetHandleByIndex_v2") orelse return null;
        var dev: Device = null;
        if (handleFn(0, &dev) != NVML_SUCCESS) return null;

        return .{
            .lib = lib,
            .device = dev,
            .getPower = lib.lookup(PowerFn, "nvmlDeviceGetPowerUsage") orelse return null,
            .getTemp = lib.lookup(TempFn, "nvmlDeviceGetTemperature") orelse return null,
            .getMem = lib.lookup(MemFn, "nvmlDeviceGetMemoryInfo") orelse return null,
            .getUtil = lib.lookup(UtilFn, "nvmlDeviceGetUtilizationRates") orelse return null,
            .getComputeProcs = lib.lookup(ComputeProcFn, "nvmlDeviceGetComputeRunningProcesses_v3") orelse return null,
            .getProcUtil = lib.lookup(ProcUtilFn, "nvmlDeviceGetProcessUtilization") orelse return null,
        };
    }

    /// Güç tüketimi — watt (milliwatt/1000).
    pub fn powerWatt(self: *const Nvml) f32 {
        var mw: c_uint = 0;
        if (self.getPower(self.device, &mw) != NVML_SUCCESS) return 0;
        return @as(f32, @floatFromInt(mw)) / 1000.0;
    }

    /// GPU die sıcaklığı — °C.
    pub fn tempC(self: *const Nvml) f32 {
        var t: c_uint = 0;
        if (self.getTemp(self.device, NVML_TEMPERATURE_GPU, &t) != NVML_SUCCESS) return 0;
        return @floatFromInt(t);
    }

    /// VRAM kullanılan/toplam (MB).
    pub fn memMb(self: *const Nvml) struct { used: u32, total: u32 } {
        var m: Memory = undefined;
        if (self.getMem(self.device, &m) != NVML_SUCCESS) return .{ .used = 0, .total = 0 };
        return .{
            .used = @intCast(m.used / 1_048_576),
            .total = @intCast(m.total / 1_048_576),
        };
    }

    /// Toplam GPU kullanımı % (nvidia-smi ile aynı API).
    pub fn utilGpu(self: *const Nvml) u32 {
        var u: Utilization = undefined;
        if (self.getUtil(self.device, &u) != NVML_SUCCESS) return 0;
        return u.gpu;
    }

    /// CUDA compute process'lerinin PID'leri. `out`'a yazar, sayıyı döndürür.
    pub fn computePids(self: *const Nvml, out: []ProcessInfo) usize {
        var count: c_uint = @intCast(out.len);
        if (self.getComputeProcs(self.device, &count, out.ptr) != NVML_SUCCESS) return 0;
        return @min(@as(usize, count), out.len);
    }

    /// Per-process kullanım örnekleri (DEC/ENC/SM). `out`'a yazar, sayıyı döndürür.
    pub fn procUtil(self: *const Nvml, out: []ProcUtilSample) usize {
        var count: c_uint = @intCast(out.len);
        if (self.getProcUtil(self.device, out.ptr, &count, 0) != NVML_SUCCESS) return 0;
        return @min(@as(usize, count), out.len);
    }
};
