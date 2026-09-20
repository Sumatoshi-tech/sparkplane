"""GPU regression: startup must load every supported sampling variant."""
import sys

import torch
from vllm.utils.jit_monitor import activate
from vllm.v1.sample.ops.topk_topp_sampler import apply_top_k_top_p
from vllm.v1.sample.ops.topk_topp_sampler import apply_top_k_top_p_pytorch

VOCAB_SIZE = 248320
MAX_ROWS = 32
logits = torch.randn(MAX_ROWS, VOCAB_SIZE, device='cuda')
top_k = torch.full((MAX_ROWS,), 20, device='cuda', dtype=torch.int32)
top_p = torch.full((MAX_ROWS,), 0.95, device='cuda')
if '--baseline' in sys.argv:
    apply_top_k_top_p(logits.clone(), top_k, top_p)
else:
    from sy_qwen38_sampling import warmup_sampling
    torch.set_default_dtype(torch.bfloat16)
    warmup_sampling(MAX_ROWS, VOCAB_SIZE, logits.device)
    torch.set_default_dtype(torch.float32)
activate(mode='error')
apply_top_k_top_p(logits[:8].clone(), top_k[:8], top_p[:8])
torch.cuda.synchronize()
for rows in range(1, MAX_ROWS + 1):
    for k, p in ((top_k[:rows], None), (None, top_p[:rows]),
                 (top_k[:rows], top_p[:rows])):
        actual = apply_top_k_top_p(logits[:rows].clone(), k, p)
        expected = apply_top_k_top_p_pytorch(
            logits[:rows].cpu(), None if k is None else k.cpu(),
            None if p is None else p.cpu(), allow_cpu_sync=True)
        actual = actual.cpu()
        if k is not None:
            torch.testing.assert_close(actual, expected)
        else:
            # The existing Triton top-p-only pivot is approximate; compare mass.
            probs = logits[:rows].cpu().softmax(dim=-1)
            mass = (probs * actual.isfinite()).sum(dim=-1)
            reference_mass = (probs * expected.isfinite()).sum(dim=-1)
            torch.testing.assert_close(mass, reference_mass, atol=0.001, rtol=0)
print('PASS: 96 sampling cases preserve filtering without late compilation')
