"""Exercise installed optimization assets without loading model weights or CUDA."""

import ast
import logging
import os
import tempfile
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from textwrap import dedent

import numpy as np
import torch

ROOT = Path("/usr/local/lib/python3.12/dist-packages")
VOCABULARY = Path("/opt/llm/draft_vocab_65536.npy")
MODEL = ROOT / "vllm/models/qwen3_8_flash_next/nvidia"


def check_vocabulary():
    ids = np.load(VOCABULARY, allow_pickle=False)
    assert ids.shape == (65536,)
    assert np.issubdtype(ids.dtype, np.integer)
    assert len(np.unique(ids)) == len(ids)
    assert int(ids.min()) >= 0 and int(ids.max()) < 248320


def check_draft_projection():
    tree = ast.parse((MODEL / "mtp.py").read_text())
    hook = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == "_dv_compute_logits")
    with tempfile.TemporaryDirectory() as directory:
        fixture = Path(directory) / "ids.npy"
        np.save(fixture, np.array([0, 2, 5], dtype=np.int64))
        namespace = {
            "torch": torch,
            "_dv_os": SimpleNamespace(environ={"VLLM_MTP_DRAFT_VOCAB": str(fixture)}),
            "_dv_logger": logging.getLogger("draft-test"),
        }
        exec(compile(ast.Module(body=[hook], type_ignores=[]), str(MODEL / "mtp.py"), "exec"), namespace)
        weights = torch.arange(24, dtype=torch.float32).reshape(6, 4)
        original = weights.clone()
        model = SimpleNamespace(lm_head=SimpleNamespace(weight=weights), logits_processor=SimpleNamespace(org_vocab_size=6))
        hidden = torch.tensor([[1.0, -2.0, 3.0, 0.5], [0.0, 1.0, 0.0, 1.0]])
        expected = torch.nn.functional.linear(hidden, weights)
        for _ in range(2):
            actual = namespace["_dv_compute_logits"](model, hidden)
            assert actual.shape == expected.shape
            torch.testing.assert_close(actual[:, [0, 2, 5]], expected[:, [0, 2, 5]])
            assert torch.isneginf(actual[:, [1, 3, 4]]).all()
            torch.testing.assert_close(weights, original, rtol=0, atol=0)


def check_mixed_metadata(prefill_tokens=3, spec_tokens=2):
    source = (ROOT / "vllm/v1/attention/backends/short_conv_attn.py").read_text()
    start = source.index("            req_group = torch.full(")
    end = source.index("\n\n            spec_state_indices_tensor", start)
    namespace = {"torch": torch, "m": SimpleNamespace(num_reqs=3), "query_start_loc": SimpleNamespace(device="unavailable-device")}
    namespace.update(query_lens_cpu=torch.tensor([prefill_tokens, 1, spec_tokens]), spec_req_idx_cpu=torch.tensor([2]), decode_req_idx_cpu=torch.tensor([1]), num_spec_decode_tokens=spec_tokens)
    with patch.object(torch.Tensor, "to", lambda tensor, *args, **kwargs: tensor):
        exec(dedent(source[start:end]), namespace)
    assert namespace["spec_token_indx"].tolist() == list(range(prefill_tokens + 1, prefill_tokens + 1 + spec_tokens))
    assert namespace["non_spec_token_indx"].tolist() == [prefill_tokens, *range(prefill_tokens)]


if __name__ == "__main__":
    assert os.getuid() == 65534, "image checks must run as the serving user"
    check_vocabulary()
    check_draft_projection()
    check_mixed_metadata()
    check_mixed_metadata(prefill_tokens=8187, spec_tokens=4)
    torch.ops.load_library("/opt/llm/kernel-det/_C_det.so")
    assert hasattr(torch.ops._C_det, "persistent_topk")
    print("vocabulary, draft projection, target preservation, mixed metadata and kernel loading: OK")
