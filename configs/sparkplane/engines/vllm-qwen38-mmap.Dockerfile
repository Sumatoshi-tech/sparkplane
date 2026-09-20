FROM vllm/vllm-openai:qwen38-flash-next@sha256:fc120ece0a388cc0aa1caad4a9f1cd92113484ab7ec2fd0efadd62585be05bf8

ARG RECIPE_REVISION=5be66376e8beaf96655f2d5682c82d538a970e66
ARG RECIPE=https://raw.githubusercontent.com/blazux/qwen3.8-Flash-DGX/${RECIPE_REVISION}
ARG SITE_PACKAGES=/usr/local/lib/python3.12/dist-packages
ARG PLE_LAYER=${SITE_PACKAGES}/vllm/models/qwen3_8_flash_next/nvidia/ple_layer.py

ADD --checksum=sha256:02adb76b789f9fccf472e922e3be7b4b2a95962ae7d7c3eb99bfeac6052dccad \
    ${RECIPE}/src/vllm_ple_mmap.py \
    ${SITE_PACKAGES}/vllm_ple_mmap.py
ADD --checksum=sha256:2652964a5084fa8c599e598f0f1d255181394a88305456a108117d4d45e34a04 ${RECIPE}/src/test_ple_mmap_cpu.py /opt/llm/checks/test_ple_mmap_cpu.py
RUN python3 /opt/llm/checks/test_ple_mmap_cpu.py

RUN chmod 0644 "${SITE_PACKAGES}/vllm_ple_mmap.py" \
    && cp "${PLE_LAYER}" "${PLE_LAYER}.orig" \
    && printf '\n\n# Pinned qwen3.8-Flash-DGX PLE mmap hook.\nfrom vllm_ple_mmap import apply as _ple_mmap_apply\n_ple_mmap_apply(Qwen3_8FlashNextNGramEmbedding)\n' >> "${PLE_LAYER}" \
    && python3 -c "import ast; ast.parse(open('${PLE_LAYER}').read())"

# Correct recurrent state copies before allowing cached-prefix restoration.
ARG FLA=${SITE_PACKAGES}/vllm/third_party/flash_linear_attention/ops
RUN sed -i 's/DEFAULT = 102400/DEFAULT = 101376  # GB10 shared-memory limit/' ${FLA}/utils.py \
    && grep -q 'DEFAULT = 101376' ${FLA}/utils.py \
    && sed -i 's/for num_warps in \[2, 4\]/for num_warps in [2]  # fla-953 Blackwell race/' ${FLA}/chunk_delta_h.py \
    && grep -Fq 'for num_warps in [2]' ${FLA}/chunk_delta_h.py
ADD --checksum=sha256:fa78ac95c3c7f0426c3a0de63441665eb2e1e2743944a3821f6b89808936be6c ${RECIPE}/src/mamba_utils_guarded.py ${SITE_PACKAGES}/vllm/v1/worker/mamba_utils.py
ADD --checksum=sha256:bcf59adbe3e317bc6300f6eecd01d2dcdb6388b7341449c74bdcee8c949b1f11 ${RECIPE}/src/patch_mamba_block_size.py /opt/llm/patch_mamba_block_size.py
RUN chmod 0644 ${SITE_PACKAGES}/vllm/v1/worker/mamba_utils.py \
    && python3 /opt/llm/patch_mamba_block_size.py ${SITE_PACKAGES}

ADD --checksum=sha256:209ddfba7ff2e6b434a4442b9b9176bd478d9d17ded94ff46a0c04a8ae42ed0e ${RECIPE}/src/patch_mtp_draft_vocab.py /opt/llm/patch_mtp_draft_vocab.py
ADD --checksum=sha256:6459e0fdc8df30e0c1d1f45be7c1b6bef0d68b0e73c073a82c52ea2a7f4b26d4 ${RECIPE}/src/draft_vocab_65536.npy /opt/llm/draft_vocab_65536.npy
RUN python3 /opt/llm/patch_mtp_draft_vocab.py ${SITE_PACKAGES}/vllm/models/qwen3_8_flash_next/nvidia/mtp.py

ARG KDET=https://raw.githubusercontent.com/jschmied/qwen38-flash-next-gb10/e0ef69d4f5575dad00d34e05479eaf4c6547bace
ADD --checksum=sha256:138cacfc5eb117f0922d53c88727e4d0dc26dcfb246c3d401fc280cfc726cc71 ${KDET}/patches/kernel-det/build_det.py /opt/llm/kernel-det/src/build_det.py
ADD --checksum=sha256:b103fbeaf7589b9468471142ad0b30012a076f93d20ba11fc5ff6dcb1ecd32a6 ${KDET}/patches/kernel-det/bindings_det.cpp /opt/llm/kernel-det/src/bindings_det.cpp
ADD --checksum=sha256:19e1d53425ea9a839445722fd1dac1c41727128eebdce84508c1bfb8592afecf ${KDET}/patches/kernel-det/topk_det.cu /opt/llm/kernel-det/src/topk_det.cu
ADD --checksum=sha256:16939700ae389750782ff5c0d5b9caef59aa0ff8b869b64ec94fa72c814910ee ${KDET}/patches/kernel-det/torch_utils.h /opt/llm/kernel-det/src/torch_utils.h
ADD --checksum=sha256:b4ef9ce298d43d6c0e6db9fcca451df20815b2cfe33791919c1ad9c0e84f0ba7 ${KDET}/patches/kernel-det/persistent_topk.cuh /opt/llm/kernel-det/src/persistent_topk.cuh
RUN cd /opt/llm/kernel-det/src && MAX_JOBS=1 DET_ARCH=121a DET_BUILD_DIR=/opt/llm/kernel-det/build python3 build_det.py \
    && cp /opt/llm/kernel-det/build/_C_det.so /opt/llm/kernel-det/_C_det.so
ADD --checksum=sha256:70905073fe3fa361030bf1cb469b74610766bdfe361419cd7df50af2561322e3 ${KDET}/tools/determinism/qsadet_patch.py /opt/llm/qsadet_patch.py
RUN VLLM_QSA_PY=${SITE_PACKAGES}/vllm/models/qwen3_8_flash_next/nvidia/ops/qsa.py python3 /opt/llm/qsadet_patch.py

ADD --checksum=sha256:4245ba4f6cce24ce10840b4db586ef4fbd361248250a15e1e5c22feb2ac4fc18 ${RECIPE}/LICENSE /opt/llm/licenses/blazux.txt
ADD --checksum=sha256:02e07067a581a25c426b339a5c9598319749393c1222f435f131667bdbb0f36a ${KDET}/LICENSE /opt/llm/licenses/jschmied.txt
COPY checks/qwen38_optimization.py /opt/llm/checks/qwen38_optimization.py
COPY patches/sha256-4ba17608fbb8908c6dfb2e796b8eb9aa4517cdcf6f261c40e8d9d8583b283e25.py /opt/llm/patches/qwen38_cpu_metadata.py
RUN python3 /opt/llm/patches/qwen38_cpu_metadata.py ${SITE_PACKAGES}/vllm/v1/attention/backends/short_conv_attn.py
RUN chmod -R a+rX /opt/llm
USER 65534
RUN HOME=/tmp python3 /opt/llm/checks/qwen38_optimization.py
USER root

COPY runtime/qwen38_sampling.py ${SITE_PACKAGES}/sy_qwen38_sampling.py
COPY checks/qwen38_sampling.py /opt/llm/checks/qwen38_sampling.py
COPY patches/sha256-6e5e0a16b41be5795a2ce0bb0e7470641947495c4179402c0298800cb3bf51e0.py /opt/llm/patches/qwen38_sampling.py
RUN python3 /opt/llm/patches/qwen38_sampling.py ${SITE_PACKAGES}/vllm/v1/worker/gpu/warmup.py \
    && chmod 0644 ${SITE_PACKAGES}/sy_qwen38_sampling.py /opt/llm/checks/qwen38_sampling.py

LABEL org.opencontainers.image.source="https://github.com/blazux/qwen3.8-Flash-DGX" \
      org.opencontainers.image.revision="${RECIPE_REVISION}" \
      sparkplane.ple-mmap="enabled"
