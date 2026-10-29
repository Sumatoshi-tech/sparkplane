"""Check serialized QSA compatibility against the image's real vLLM config."""
import copy
import unittest

from vllm.models.qwen4_exp.config import Qwen4ExpConfig, Qwen4ExpTextConfig


def checkpoint_config():
    # RedHatAI c8f2fb1b9869f686b214782036123b10ff96d14a text geometry.
    return {
        "num_hidden_layers": 48,
        "hidden_size": 2560,
        "head_dim": 256,
        "partial_rotary_factor": 0.25,
        "layer_types": ["linear_attention"] * 3 + ["qwen_sparse_attention"],
        "indexer_n_heads": 4,
        "indexer_kv_heads": 1,
        "indexer_head_dim": 128,
        "indexer_budget": 2048,
        "indexer_compress_ratio": 4,
        "ple_layer_ids": [2],
        "ple_embed_dim": 2560,
    }


class SerializedQSATests(unittest.TestCase):
    def options(self):
        options = checkpoint_config()
        options["layer_types"] *= 12
        return options

    def test_serialized_sparse_attention_retains_qsa_geometry(self):
        options = self.options()
        original = copy.deepcopy(options)
        config = Qwen4ExpConfig(text_config=options).get_text_config()
        self.assertEqual(options, original)
        self.assertEqual(config.layer_types, (["linear_attention"] * 3 + ["full_attention"]) * 12)
        # The pinned model chooses QSA for precisely these layers when its
        # indexer fields are present; no indexer or attention geometry changes.
        self.assertEqual([i for i, kind in enumerate(config.layer_types) if kind == "full_attention"], list(range(3, 48, 4)))
        for field in ["indexer_n_heads", "indexer_kv_heads", "indexer_head_dim", "indexer_budget", "indexer_compress_ratio", "head_dim", "partial_rotary_factor", "hidden_size", "ple_embed_dim", "ple_layer_ids"]:
            self.assertEqual(getattr(config, field), original[field])
        self.assertEqual(config.layers_block_type.count("attention"), 12)
        self.assertEqual(config.layers_block_type.count("linear_attention"), 36)

    def test_legacy_layer_labels_are_unchanged(self):
        options = self.options()
        options["layer_types"] = (["linear_attention"] * 3 + ["full_attention"]) * 12
        config = Qwen4ExpTextConfig(**options)
        self.assertEqual(config.layer_types, options["layer_types"])

    def test_unknown_layer_label_is_still_rejected(self):
        options = self.options()
        options["layer_types"][3] = "unknown_attention"
        with self.assertRaises(ValueError):
            Qwen4ExpTextConfig(**options)

    def test_invalid_qsa_geometry_is_still_rejected(self):
        options = self.options()
        options["indexer_budget"] = 3
        with self.assertRaises(ValueError):
            Qwen4ExpTextConfig(**options)


if __name__ == "__main__":
    unittest.main()
