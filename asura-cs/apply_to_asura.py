#!/usr/bin/env python3
"""Apply a profile's spawn points to ASURA and restart its worlds:  python apply_to_asura.py [profile-folder]"""
import sys

from asura_cs.ui.deploy_window import main

if __name__ == "__main__":
    sys.exit(main())
