#include "libs/gb_apu/Gb_Apu.h"
#include "libs/gb_apu/Multi_Buffer.h" // or "Stereo_Buffer.h"
#include <stdint.h>
#include <stdlib.h>

extern "C" {

    struct ApuCtx {
        Gb_Apu apu;
        Stereo_Buffer buf; 
        int sample_rate;
    };

    ApuCtx* apu_new(int sample_rate) {
        ApuCtx* ctx = new ApuCtx();
        ctx->sample_rate = sample_rate;

        // Configure buffer
        ctx->buf.set_sample_rate(sample_rate, 1000); // approximate latency (ms)
        ctx->buf.clock_rate(4194304);                // GB CPU clock

        // Connect APU to buffer
        ctx->apu.output(ctx->buf.center(), ctx->buf.left(), ctx->buf.right());

        // Optional EQ (softens high frequencies)
        // ctx->apu.treble_eq( Blip_eq_t( -24.0, 8800, 160 ) );

        return ctx;
    }

    void apu_delete(ApuCtx* ctx) {
        delete ctx;
    }

    void apu_reset(ApuCtx* ctx) {
        ctx->apu.reset();
        ctx->buf.clear();
    }

    void apu_write(ApuCtx* ctx, uint32_t time_clocks, uint16_t addr, uint8_t data) {
        ctx->apu.write_register(time_clocks, addr, data);
    }

    void apu_end_frame(ApuCtx* ctx, uint32_t frame_clocks) {
        ctx->apu.end_frame(frame_clocks);
        ctx->buf.end_frame(frame_clocks);
    }

    // Reads interleaved L,R samples in i16. Returns the number of samples (frames*2)
    int apu_read_samples(ApuCtx* ctx, int16_t* out, int max_samples_stereo) {
        // max_samples_stereo is the number of interleaved samples (L,R,L,R,...)
        long n = ctx->buf.read_samples(out, max_samples_stereo);
        return (int)n;
    }

    // NR52 master enable: if you turn it off, it's a good idea to reset/mute
    void apu_master_enable(ApuCtx* ctx, int enable) {
        // In Gb_Apu there is no direct “power”; we simulate it:
        if (!enable) {
            ctx->apu.reset();  // turn everything off
            ctx->buf.clear();
        }
    }

} // extern "C"
