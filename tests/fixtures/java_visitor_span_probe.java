package com.test;

import java.lang.foreign.MemoryLayout;
import java.lang.reflect.Field;

// `javac` type-checks `decodeContext` but cannot see whether the offsets it reads from are the
// ones `#[repr(C)]` actually puts the fields at — a layout that is wrong by a few bytes of
// padding compiles perfectly and then reads garbage at run time. This probe loads the generated
// bridge and checks the derived layout against the offsets `#[repr(C)]` gives
// `(*const c_char, u8, i32, i16, *const c_char)` on a 64-bit target. ~keep
public final class SpanProbe {

    public static void main(final String[] args) throws Exception {
        expectOffset("CTX_OFFSET_LABEL", 0L);
        expectOffset("CTX_OFFSET_SEVERITY", 8L);
        expectOffset("CTX_OFFSET_ACTIVE", 12L);
        expectOffset("CTX_OFFSET_OFFSET", 16L);
        expectOffset("CTX_OFFSET_NOTE", 24L);

        final Field declared = VisitorBridge.class.getDeclaredField("CTX_LAYOUT");
        declared.setAccessible(true);
        final long size = ((MemoryLayout) declared.get(null)).byteSize();
        if (size != 32L) {
            throw new AssertionError("CTX_LAYOUT.byteSize() was " + size + ", expected 32");
        }

        expectNoOffset("CTX_OFFSET_WEIGHT");
        expectNoOffset("CTX_OFFSET_TAGS");
    }

    private static void expectOffset(final String name, final long expected) throws Exception {
        final Field declared = VisitorBridge.class.getDeclaredField(name);
        declared.setAccessible(true);
        final long actual = declared.getLong(null);
        if (actual != expected) {
            throw new AssertionError(name + " was " + actual + ", expected " + expected);
        }
    }

    private static void expectNoOffset(final String name) {
        try {
            VisitorBridge.class.getDeclaredField(name);
            throw new AssertionError(name + " exists, but the C struct does not carry that field");
        } catch (NoSuchFieldException expected) {
            // The field has no C representation, so it must have no offset constant.
        }
    }
}
