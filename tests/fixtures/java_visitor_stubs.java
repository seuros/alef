package com.test;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;

// Stand-ins for the generated records, which carry Jackson annotations and so cannot be compiled
// without the Jackson jars on the classpath. The generated method under test only needs their
// shape, never their serialization behaviour. ~keep
record VisitContext(String path) {}

record WorkConfig(Callback hook, String mode) {}

record WorkResult(String text) {}

final class VisitorBridge implements AutoCloseable {
    private final Arena arena = Arena.ofConfined();

    VisitorBridge(final Callback visitor) {
    }

    MemorySegment callbacksStruct() {
        return MemorySegment.NULL;
    }

    void rethrowVisitorError() throws Throwable {
    }

    @Override
    public void close() {
        arena.close();
    }
}
