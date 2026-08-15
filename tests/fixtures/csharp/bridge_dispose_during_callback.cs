using System;
using System.Reflection;
using System.Runtime.InteropServices;
using Test;

var bridge = new TextBackendBridge(new Backend());
var flags = BindingFlags.Instance | BindingFlags.NonPublic;
var increment = typeof(TextBackendBridge).GetMethod("IncrementCallbackRef", flags)!;
var decrement = typeof(TextBackendBridge).GetMethod("DecrementCallbackRef", flags)!;
var handleField = typeof(TextBackendBridge).GetField("_implHandle", flags)!;
var bridgeId = bridge._bridgeId;

lock (TextBackendBridge._registryLock)
{
    TextBackendBridge._bridgeRegistry[bridgeId] = bridge;
}

increment.Invoke(bridge, null);
bridge.Dispose();
AssertResourcesAlive(bridge, handleField, "public Dispose while native owns bridge");

TextBackendBridge.FreeUserData(bridgeId);
AssertResourcesAlive(bridge, handleField, "native release while callback is active");

decrement.Invoke(bridge, null);
if (bridge._vtable != IntPtr.Zero)
    throw new InvalidOperationException("vtable survived final callback release");
if (((GCHandle)handleField.GetValue(bridge)!).IsAllocated)
    throw new InvalidOperationException("GCHandle survived final callback release");

lock (TextBackendBridge._registryLock)
{
    if (TextBackendBridge._bridgeRegistry.ContainsKey(bridgeId))
        throw new InvalidOperationException("bridge registry retained released bridge");
}

static void AssertResourcesAlive(TextBackendBridge bridge, FieldInfo handleField, string phase)
{
    if (bridge._vtable == IntPtr.Zero)
        throw new InvalidOperationException($"vtable released during {phase}");
    if (!((GCHandle)handleField.GetValue(bridge)!).IsAllocated)
        throw new InvalidOperationException($"GCHandle released during {phase}");
}

sealed class Backend : ITextBackend { }
