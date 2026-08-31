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
from typing import TYPE_CHECKING, Protocol, TypedDict, cast

if TYPE_CHECKING:
    from collections.abc import Callable

CLASS_SHAPE = "class"
DICT_SHAPE = "dict"
CONTROLS = ("map", "list", "index")


class ProbeVisitor(Protocol):
    """The two recorders every generated `_TestVisitor` exposes.

    Declared as a Protocol rather than typed `Any` so a rename on the generator side surfaces
    here as a type error instead of an `AttributeError` at run time.
    """

    context_errors: list[str]
    context_reads: int


class ShapeReport(TypedDict):
    """What one context shape produced: the observed controls and the probe's own record."""

    controls: dict[str, bool]
    errors: list[str]
    reads: int


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
    namespace: dict[str, object] = {name: f"value-of-{name}" for name in attributes}
    for name in methods:
        namespace[name] = _zero_arg_method(name)
    return type("GeneratedContextClass", (), namespace)()


def _zero_arg_method(name: str) -> Callable[[object], str]:
    """Build a bound zero-argument method, the shape `#[pymethods]` publishes."""

    def method(_self: object) -> str:
        return f"result-of-{name}"

    return method


def build_dict_context(attributes: list[str], methods: list[str]) -> dict[str, str]:
    """The bridge's fallback ``PyDict``, keyed by the very names the generator declared."""
    return {name: f"value-of-{name}" for name in [*attributes, *methods]}


def call_callback(visitor: ProbeVisitor, callback: str, ctx: object) -> None:
    """Invoke the generated callback, filling its non-context parameters with placeholders."""
    method = getattr(visitor, callback)
    extra = len(inspect.signature(method).parameters) - 1
    method(ctx, *["placeholder"] * extra)


def run_shape(visitor_type: Callable[[], ProbeVisitor], callback: str, ctx: object) -> ShapeReport:
    """Run one callback against one context shape and report what the probe recorded."""
    visitor = visitor_type()
    call_callback(visitor, callback, ctx)
    return {
        "controls": observe_controls(ctx),
        "errors": list(visitor.context_errors),
        "reads": visitor.context_reads,
    }


def load_visitor_type(class_source: str) -> Callable[[], ProbeVisitor]:
    """Execute the generated class source and hand back its `_TestVisitor` constructor."""
    namespace: dict[str, object] = {}
    exec(compile(class_source, "<alef-generated>", "exec"), namespace)  # noqa: S102
    return cast("Callable[[], ProbeVisitor]", namespace["_TestVisitor"])


def check_controls(class_report: ShapeReport, dict_report: ShapeReport) -> list[str]:
    """The two shapes must separate on every control, or neither result means anything."""
    failures: list[str] = []
    for control in CONTROLS:
        if class_report["controls"][control]:
            failures.append(f"control {control}: the class-shaped context answered it; the two shapes are not distinct")
        if not dict_report["controls"][control]:
            failures.append(f"control {control}: the dict-shaped context did not answer it; the control is inert")
    return failures


def check_verdicts(class_report: ShapeReport, dict_report: ShapeReport) -> list[str]:
    """The probe must accept the declared class and reject the bridge's fallback dict."""
    failures: list[str] = []

    if class_report["errors"]:
        failures.append(f"class-shaped context was rejected: {class_report['errors']}")
    if class_report["reads"] != 1:
        failures.append(f"class-shaped context was never probed: reads={class_report['reads']}")

    if not dict_report["errors"]:
        failures.append(
            "dict-shaped context was accepted: every declared name is also a key on the dict, so "
            "the name probes pass and only a shape check can reject it"
        )
    elif not all("dict-shaped" in error for error in dict_report["errors"]):
        failures.append(f"dict rejection did not name the shape: {dict_report['errors']}")
    if dict_report["reads"] != 1:
        failures.append(f"dict-shaped context was never probed: reads={dict_report['reads']}")

    return failures


def main() -> int:
    """Read the job, run both shapes, and report every expectation that did not hold."""
    job = json.load(sys.stdin)
    attributes: list[str] = job["attributes"]
    methods: list[str] = job["methods"]
    callback: str = job["callback"]

    visitor_type = load_visitor_type(job["class_source"])
    class_report = run_shape(visitor_type, callback, build_class_context(attributes, methods))
    dict_report = run_shape(visitor_type, callback, build_dict_context(attributes, methods))

    failures = check_controls(class_report, dict_report) + check_verdicts(class_report, dict_report)

    json.dump(
        {CLASS_SHAPE: class_report, DICT_SHAPE: dict_report, "failures": failures},
        sys.stdout,
        indent=2,
        sort_keys=True,
    )
    sys.stdout.write("\n")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
