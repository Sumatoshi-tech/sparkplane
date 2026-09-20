"""Keep mixed-request token ordering on CPU; only transfer the permutation."""
import hashlib
import sys
from pathlib import Path

path = Path(sys.argv[1])
source = path.read_text()
assert hashlib.sha256(source.encode()).hexdigest() == "985fd9f320eff615f2bc4b4d5576cd7f301a751380709979a251f92101bda14d", "unexpected short-conv source"
start = source.index("            req_group = torch.full(")
end = source.index("\n            spec_token_indx", start)
original = source[start:end]
updated = (
    original.replace("device=query_start_loc.device", 'device="cpu"')
    .replace("req_group[spec_req_idx]", "req_group[spec_req_idx_cpu]")
    .replace("decode_req_idx_cpu.to(query_start_loc.device)", "decode_req_idx_cpu")
    .replace("req_group, query_lens)", "req_group, query_lens_cpu)")
    .replace("torch.argsort(token_group, stable=True)", "torch.argsort(token_group, stable=True).to(query_start_loc.device)")
)
path.write_text(source[:start] + updated + source[end:])
