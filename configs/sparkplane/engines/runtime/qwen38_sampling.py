"""Load the finite top-k/top-p sampler variants before serving requests."""
import torch
from vllm.v1.sample.ops.topk_topp_sampler import apply_top_k_top_p


def warmup_sampling(max_rows, vocab_size, device):
    logits = torch.linspace(-1, 1, vocab_size, device=device, dtype=torch.float32).expand(max_rows, -1)
    top_k = torch.full((max_rows,), 20, device=device, dtype=torch.int32)
    top_p = torch.full((max_rows,), 0.95, device=device, dtype=torch.float32)
    for rows in range(1, max_rows + 1):
        for k, p in ((top_k[:rows], None), (None, top_p[:rows]),
                     (top_k[:rows], top_p[:rows])):
            apply_top_k_top_p(logits[:rows].clone(), k, p)
    torch.cuda.synchronize(device)
