#!/usr/bin/env python3
"""Supported repository-root entrypoint for the seat installer."""
import os
import runpy

installer = os.path.join(os.path.dirname(os.path.abspath(__file__)), "seats", "install.py")
runpy.run_path(installer, run_name="__main__")
