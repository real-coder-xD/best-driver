"""Type stub for the compiled Rust/PyO3 extension submodule (`best_driver.best_driver`).

Re-exports the typed definitions from `best_driver.__init__.pyi` so both VS Code
(Pylance / Pyright) and PyCharm resolve full signatures, TypedDicts, and hover
docstrings without falling back to binary `.pyd`/`.so` skeletons.
"""

from . import (
    Browser as Browser,
    BrowserContext as BrowserContext,
    Element as Element,
    EventStream as EventStream,
    Tab as Tab,
    connect as connect,
    start as start,
)

__all__ = [
    "Browser",
    "BrowserContext",
    "Element",
    "EventStream",
    "Tab",
    "connect",
    "start",
]
