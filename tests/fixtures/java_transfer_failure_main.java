package com.test;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.concurrent.atomic.AtomicReference;
import sun.misc.Unsafe;

public final class TransferFailureMain {
    private TransferFailureMain() {}

    public static void main(String[] args) throws Exception {
        System.setProperty("alef.fail.transfer", "true");
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment expected = arena.allocate(1);
            if (args.length != 1) throw new IllegalArgumentException("expected opaque or service");
            if ("opaque".equals(args[0])) verifyOpaque(expected);
            else if ("service".equals(args[0])) verifyService(7L);
            else throw new IllegalArgumentException("unknown scenario: " + args[0]);
        } finally {
            System.clearProperty("alef.fail.transfer");
        }
    }

    private static void verifyOpaque(MemorySegment expected) throws Exception {
        Resource resource = new Resource(expected);
        expectAllocationFailure(Resource.class.getDeclaredMethod("takeHandle"), resource);
        AtomicReference<Throwable> failure = new AtomicReference<>();
        Thread closer = new Thread(() -> closeOpaque(resource, failure));
        closer.setDaemon(true);
        closer.start();
        closer.join(1_000);
        if (closer.isAlive()) throw new AssertionError("opaque close remained blocked after allocation failure");
        if (failure.get() != null) throw new AssertionError("opaque close failed", failure.get());
        if (NativeLib.freeCalls != 1) throw new AssertionError("opaque owner was not closeable after allocation failure");
    }

    private static void closeOpaque(Resource resource, AtomicReference<Throwable> failure) {
        try {
            resource.close();
        } catch (Throwable error) {
            failure.set(error);
        }
    }

    private static void verifyService(long expected) throws Exception {
        TestService service = (TestService) unsafe().allocateInstance(TestService.class);
        Field owner = TestService.class.getDeclaredField("ownerHandle");
        owner.setAccessible(true);
        owner.setLong(service, expected);
        expectAllocationFailure(privateMethod("takeOwnerHandle"), service);
        AtomicReference<Long> detached = new AtomicReference<>();
        AtomicReference<Throwable> failure = new AtomicReference<>();
        Thread closer = new Thread(() -> takeServiceForClose(service, detached, failure));
        closer.setDaemon(true);
        closer.start();
        closer.join(1_000);
        if (closer.isAlive()) throw new AssertionError("service close remained blocked after allocation failure");
        if (failure.get() != null) throw new AssertionError("service close failed", failure.get());
        if (detached.get() == null || expected != detached.get()) throw new AssertionError("service owner was not closeable after allocation failure");
    }

    private static void takeServiceForClose(
            TestService service, AtomicReference<Long> detached, AtomicReference<Throwable> failure) {
        try {
            detached.set((long) privateMethod("takeOwnerHandleForClose").invoke(service));
        } catch (Throwable error) {
            failure.set(error);
        }
    }

    private static void expectAllocationFailure(Method method, Object receiver) throws Exception {
        method.setAccessible(true);
        try {
            method.invoke(receiver);
            throw new AssertionError("transfer allocation unexpectedly succeeded");
        } catch (InvocationTargetException error) {
            if (!(error.getCause() instanceof OutOfMemoryError)) throw error;
        }
    }

    private static Method privateMethod(String name) throws Exception {
        Method method = TestService.class.getDeclaredMethod(name);
        method.setAccessible(true);
        return method;
    }

    private static Unsafe unsafe() throws Exception {
        Field field = Unsafe.class.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        return (Unsafe) field.get(null);
    }
}
