"""Wire finite sampler warmup into the exact preview vLLM startup."""
import hashlib
import sys
from pathlib import Path

target = Path(sys.argv[1])
source = target.read_text()
expected = '461eeca4751f6c59598c0e9fca501ebdf62aefd517ed060221a81d2ad4b2a830'
assert hashlib.sha256(source.encode()).hexdigest() == expected, 'unrecognized warmup source'
anchor = '    num_spec_steps = model_runner.num_speculative_steps\n'
assert source.count(anchor) == 1
replacement = anchor + '''    from sy_qwen38_sampling import warmup_sampling
    if not model_runner.is_pooling_model:
        warmup_sampling(
            model_runner.scheduler_config.max_num_seqs * (num_spec_steps + 1),
            model_runner.model_config.get_vocab_size(), model_runner.device)
        logger.info("Completed finite top-k/top-p sampler startup coverage")
'''
target.write_text(source.replace(anchor, replacement))
