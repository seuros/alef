package com.test;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.reflect.Method;

public final class UpcallFailureMain {
    private UpcallFailureMain() {}

    public static void main(String[] args) throws Exception {
        Method invoke = TestService.class.getDeclaredMethod(
                "invokeHandlerWithMarshal", MemorySegment.class, MemorySegment.class,
                Callable.class, Arena.class);
        invoke.setAccessible(true);
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment request = arena.allocateFrom("request");
            assertNull(invoke, request, ignored -> { throw new IllegalStateException("handler"); }, arena);
            assertNull(invoke, request, ignored -> null, arena);
            assertNull(invoke, request, ignored -> "allocation", arena);
        }
    }

    private static void assertNull(Method invoke, MemorySegment request, Callable handler, Arena arena)
            throws Exception {
        Object result = invoke.invoke(null, MemorySegment.NULL, request, handler, arena);
        if (!MemorySegment.NULL.equals(result)) throw new AssertionError("failure escaped as a response pointer");
    }
}
