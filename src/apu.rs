use std::ffi::c_void;
use std::os::raw::{c_int, c_uint, c_ushort, c_uchar};
use std::ptr::NonNull;

#[repr(C)]
struct ApuCtxOpaque(c_void);

#[link(name = "gb_apu")]
unsafe extern "C" {
    fn apu_new(sample_rate: c_int) -> *mut ApuCtxOpaque;
    fn apu_delete(ctx: *mut ApuCtxOpaque);
    // fn apu_reset(ctx: *mut ApuCtxOpaque);
    fn apu_write(ctx: *mut ApuCtxOpaque, time_clocks: c_uint, addr: c_ushort, data: c_uchar);
    fn apu_end_frame(ctx: *mut ApuCtxOpaque, frame_clocks: c_uint);
    fn apu_read_samples(ctx: *mut ApuCtxOpaque, out: *mut i16, max_samples_stereo: c_int) -> c_int;
    fn apu_master_enable(ctx: *mut ApuCtxOpaque, enable: c_int);
}

pub struct APU {
    ctx: NonNull<ApuCtxOpaque>,
    // accumulated clock count (CPU clocks) for write timestamps
    clock_acc: u32,
}

impl APU {
    pub fn new(sample_rate: u32) -> Self {
        let ptr = unsafe { apu_new(sample_rate as c_int) };
        let ctx = NonNull::new(ptr).expect("apu_new failed");
        Self { ctx, clock_acc: 0 }
    }

    // pub fn reset(&mut self) { unsafe { apu_reset(self.ctx.as_ptr()) } }

    /// Call for each write to NRxx / Wave RAM (0xFF10..0xFF26, 0xFF30..0xFF3F)
    pub fn write(&mut self, addr: u16, data: u8) {
        unsafe { apu_write(self.ctx.as_ptr(), self.clock_acc, addr, data) }
    }

    /// Advances the APU time (in **CPU clocks**, not m-cycles)
    pub fn advance_clocks(&mut self, clocks: u32) {
        self.clock_acc = self.clock_acc.wrapping_add(clocks);
    }

    /// Closes a logical “frame” and flushes internal samples
    pub fn end_frame(&mut self, clocks: u32) {
        unsafe { apu_end_frame(self.ctx.as_ptr(), clocks) }
        // reduce accumulator to prevent overflow:
        self.clock_acc = self.clock_acc.wrapping_sub(clocks);
    }

    /// Reads interleaved stereo samples i16 (L,R,L,R...)
    pub fn read_samples(&mut self, out: &mut [i16]) -> usize {
        unsafe { apu_read_samples(self.ctx.as_ptr(), out.as_mut_ptr(), out.len() as c_int) as usize }
    }

    pub fn master_enable(&mut self, enable: bool) {
        unsafe { apu_master_enable(self.ctx.as_ptr(), if enable {1} else {0}) }
    }
}

impl Drop for APU {
    fn drop(&mut self) { unsafe { apu_delete(self.ctx.as_ptr()) } }
}
