#!/usr/bin/env python3
"""Emscripten cdylib link, keeping the worker ABI and Rust runtime exports."""
import argparse
import json
import os
import subprocess
parser = argparse.ArgumentParser()
parser.add_argument('-o', required=True)
args, rest = parser.parse_known_args()
exports = {'_dfs_prepare', '_dfs_solve', '_dfs_resume', '_malloc', '_free'}
cleaned = []
for arg in rest:
    if arg == '-sSIDE_MODULE=2':
        continue
    if arg.startswith('EXPORTED_FUNCTIONS='):
        value = arg.split('=', 1)[1]
        exports.update(json.loads(value) if value.startswith('[') else value.split(','))
        if cleaned and cleaned[-1] == '-s':
            cleaned.pop()
    else:
        cleaned.append(arg)
subprocess.check_call(['emcc', '-o', os.path.splitext(args.o)[0] + '.js', *cleaned,
                       '-s', 'EXPORTED_FUNCTIONS=' + ','.join(sorted(exports))])
