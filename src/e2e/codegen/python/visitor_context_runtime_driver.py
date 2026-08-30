"""Execute alef's generated visitor-context probe against both context shapes.

Reads one JSON job on stdin and writes one JSON report on stdout. The job carries the REAL
generated `_TestVisitor` source (produced by the Python e2e generator, not written here) plus
the context surface the generator declared for it; this driver only supplies the two runtime
objects the probe has to tell apart and checks what the probe recorded.

The two objects are the two shapes the PyO3 visitor bridge can actually hand a callback:

* class-shaped -- the generated ``#[pyclass]`` the ``.pyi`` stub declares. Attributes are real
  attributes and declared zero-argument methods are real methods. It is not a mapping, is not
  iterable and is not subscriptable.
* dict-shaped -- the bridge's fallback ``PyDict``. Its keys are deliberately the *same* names the
  generator declared, so every name-based probe (``getattr`` and ``getattr(...)()``) is satisfied
  by the dict's own API and only a shape check can tell the two apart.

Before running the probe the driver observes the MAP, LIST and INDEX controls on both objects and
fails if they do not separate the shapes -- without that, a probe that recorded nothing at all
would look identical to a probe that correctly saw a class.

Exits non-zero with a diagnostic on stdout when any expectation fails.
"""

from __future__ import annotations

import inspect
import json
import sys
from typing import Any

CLASS_SHAPE = "class"
DICT_SHAPE = "dict"


def observe_controls(ctx: object) -> dict[str, bool]:
    """Observe the three shape controls on a real object rather than asserting its type."""
    controls = {"map": isinstance(ctx, dict)}

    try:
        list(iter(ctx))  # type: ignore[call-overload]
    except TypeError:
        controls["list"] = False
    else:
        controls["list"] = True

    try:
        ctx["probe"]  # type: ignore[index]
    except (TypeError, AttributeError):
        controls["index"] = False
    except KeyError:
        controls["index"] = True
    else:
        controls["index"] = True

    return controls


def build_class_context(attributes: list[str], methods: list[str]) -> object:
    """A stand-in for the generated ``#[pyclass]``: real attributes, real zero-arg methods."""
    namespace: dict[str, Any] = {name: f"value-of-{name}" for name in attributes}
    for name in methods:
        namespace[name] = _zero_arg_method(name)
    return type("GeneratedContextClass", (), namespace)()


def _zero_arg_method(name: str) -> Any:
    """Build a bound zero-argument method, the shape `#[pymethods]` publishes."""

    def method(_self: object) -> str:
        return f"result-of-{name}"

    return method


def build_dict_context(attributes: list[str], methods: list[str]) -> dict[str, str]:
    """The bridge's fallback ``PyDict``, keyed by the very names the generator declared."""
    return {name: f"value-of-{name}" for name in [*attributes, *methods]}


def call_callback(visitor: object, callback: str, ctx: object) -> None:
    """Invoke the generated callback, filling its non-context parameters with placeholders."""
    method = getattr(visitor, callback)
    extra = len(inspect.signature(method).parameters) - 1
    method(ctx, *["placeholder"] * extra)


def run_shape(visitor_type: type, callback: str, ctx: object) -> dict[str, Any]:
    """Run one callback against one context shape and report what the probe recorded."""
    visitor = visitor_type()
    call_callback(visitor, callback, ctx)
    return {
        "controls": observe_controls(ctx),
        "errors": list(visitor.context_errors),
        "reads": visitor.context_reads,
    }


def main() -> int:
    """Read the job, run both shapes, and report every expectation that did not hold."""
    job = json.load(sys.stdin)
    attributes: list[str] = job["attributes"]
    methods: list[str] = job["methods"]

    namespace: dict[str, Any] = {}
    exec(compile(job["class_source"], "<alef-generated>", "exec"), namespace)  # noqa: S102
    visitor_type = namespace["_TestVisitor"]

    report: dict[str, Any] = {
        CLASS_SHAPE: run_shape(visitor_type, job["callback"], build_class_context(attributes, methods)),
        DICT_SHAPE: run_shape(visitor_type, job["callback"], build_dict_context(attributes, methods)),
    }

    failures: list[str] = []

    class_controls = report[CLASS_SHAPE]["controls"]
    dict_controls = report[DICT_SHAPE]["controls"]
    for control in ("map", "list", "index"):
        if class_controls[control]:
            failures.append(f"control {control}: the class-shaped context answered it; the two shapes are not distinct")
        if not dict_controls[control]:
            failures.append(f"control {control}: the dict-shaped context did not answer it; the control is inert")

    if report[CLASS_SHAPE]["errors"]:
        failures.append(f"class-shaped context was rejected: {report[CLASS_SHAPE]['errors']}")
    if report[CLASS_SHAPE]["reads"] != 1:
        failures.append(f"class-shaped context was never probed: reads={report[CLASS_SHAPE]['reads']}")

    if not report[DICT_SHAPE]["errors"]:
        failures.append(
            "dict-shaped context was accepted: every declared name is also a key on the dict, so "
            "the name probes pass and only a shape check can reject it"
        )
    elif not all("dict-shaped" in error for error in report[DICT_SHAPE]["errors"]):
        failures.append(f"dict rejection did not name the shape: {report[DICT_SHAPE]['errors']}")
    if report[DICT_SHAPE]["reads"] != 1:
        failures.append(f"dict-shaped context was never probed: reads={report[DICT_SHAPE]['reads']}")

    report["failures"] = failures
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
