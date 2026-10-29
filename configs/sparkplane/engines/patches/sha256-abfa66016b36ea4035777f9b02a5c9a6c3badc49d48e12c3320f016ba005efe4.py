"""Normalize the serialized QSA layer label in the pinned Qwen4Exp runtime."""
import hashlib
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_bytes()
if hashlib.sha256(source).hexdigest() != "71230ef5e53e016416f54ae96b9f8b4e2c10a6311d8e6035ac063ac48fea2471":
    raise SystemExit("unsupported Qwen4Exp config; refusing to patch")
old = b"        super().__init__(layer_types=layer_types, **kwargs)\n"
new = b"""        # This runtime selects QSA from indexer_n_heads on full_attention layers.
        # Normalize only the checkpoint label; retain every QSA geometry field.
        if layer_types is not None:
            layer_types = [
                "full_attention" if kind == "qwen_sparse_attention" else kind
                for kind in layer_types
            ]
        super().__init__(layer_types=layer_types, **kwargs)
"""
if source.count(old) != 1:
    raise SystemExit("Qwen4Exp config patch anchor is ambiguous")
path.write_bytes(source.replace(old, new, 1))
