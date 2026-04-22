#!/usr/bin/env bash
set -euo pipefail

exec gvproxy -listen vsock://:1024 -listen unix:///tmp/network.sock
