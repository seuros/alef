package com.example;

import java.lang.foreign.Arena;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.concurrent.atomic.AtomicBoolean;
import sun.reflect.ReflectionFactory;

/// Drives the generated TestService owner-gate logic (borrowOwnerHandle / close)
/// without a loaded native library: the real generated constructor and the final
/// `close()` free-downcall both require a real FFI symbol, which this harness has
/// no way to provide (no compiled native crate is built for a plain `cargo test`
/// run of the alef generator). ReflectionFactory bypasses the FFI constructor so
/// this can instantiate the class directly; the `final` fields the real
/// constructor would normally initialize (`arena`, `ownerMutationLock`) are set
/// by hand, and `ownerHandle` is reset to 0 before `close()` so the free-downcall
/// branch (`detached != 0`) is never taken — the exercised path stays entirely
/// inside generated Java. ~keep
public final class ServiceGateMain {
    private ServiceGateMain() {}

    public static void main(String[] args) throws Exception {
        Constructor<Object> objectCtor = Object.class.getDeclaredConstructor();
        Constructor<?> bypassCtor = ReflectionFactory.getReflectionFactory()
                .newConstructorForSerialization(TestService.class, objectCtor);
        bypassCtor.setAccessible(true);
        TestService service = (TestService) bypassCtor.newInstance();

        setField(service, "arena", Arena.ofShared());
        setField(service, "ownerMutationLock", new Object());

        Field ownerHandle = TestService.class.getDeclaredField("ownerHandle");
        ownerHandle.setAccessible(true);
        ownerHandle.setLong(service, 1L);

        Method borrow = TestService.class.getDeclaredMethod("borrowOwnerHandle");
        borrow.setAccessible(true);
        AutoCloseable lease = (AutoCloseable) borrow.invoke(service);

        AtomicBoolean leaseReleased = new AtomicBoolean(false);

        Thread releaser = new Thread(() -> {
            try {
                Thread.sleep(200);
                lease.close();
                leaseReleased.set(true);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        });
        releaser.start();

        // ownerHandle must read 0 once close() unblocks, or it takes the
        // free-downcall branch (no real native library is loaded here).
        // activeOwnerBorrows — not ownerHandle — is what makes close() block.
        ownerHandle.setLong(service, 0L);
        service.close();
        boolean closedAfterRelease = leaseReleased.get();
        releaser.join();

        if (!closedAfterRelease) {
            throw new AssertionError("close() returned before the outstanding lease released");
        }
        System.out.println("service owner gate blocked close() until release, as expected");
    }

    private static void setField(Object target, String name, Object value) throws Exception {
        Field field = TestService.class.getDeclaredField(name);
        field.setAccessible(true);
        field.set(target, value);
    }
}
