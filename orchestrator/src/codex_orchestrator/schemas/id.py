"""ID generation utility."""
from __future__ import annotations

import secrets
import string

_ALPHABET = string.ascii_lowercase + string.digits


def generate_id(prefix: str, length: int = 12) -> str:
    """Generate a prefixed random ID like 'r_a8kx3mp2vq1n'."""
    body = "".join(secrets.choice(_ALPHABET) for _ in range(length))
    return f"{prefix}_{body}"
