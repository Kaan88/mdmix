#!/usr/bin/env python3
"""Double-click launcher for the G6 GUI without installing the package (uses ./src)."""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "src"))

from g6_cli.gui import main  # noqa: E402

main()
