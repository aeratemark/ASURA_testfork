#!/usr/bin/env python3
"""Start the ASURA Construction Set:  python run_editor.py [profile-folder]"""
import sys

from asura_cs.ui.main_window import run_app

if __name__ == "__main__":
    sys.exit(run_app(sys.argv))
