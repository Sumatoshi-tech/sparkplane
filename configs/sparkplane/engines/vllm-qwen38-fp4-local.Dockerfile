FROM eugr/spark-vllm-b12x@sha256:03a956544e9f9b4b7ba91690469fbe7848bdbef29a7d2e0f5788368467bb3595

ARG LOADER=/usr/local/lib/python3.12/dist-packages/flashinfer/experimental/b12x/loader
COPY patches/sha256-7dc4adb2a6d401da7d92bb45016a1dc68bdfad6b3b4d06b0c9920151d12152f9.py /opt/llm/patches/checkpoint_pread.py
RUN python3 /opt/llm/patches/checkpoint_pread.py ${LOADER}
COPY checks/qwen38_fp4_checkpoint.py /opt/llm/checks/qwen38_fp4_checkpoint.py
COPY patches/sha256-33d669fc7b0e3761f6ef213b11789196fca52801c7ceb991bb4b271489aac1bc.py /opt/llm/patches/ple_pread.py
RUN python3 /opt/llm/patches/ple_pread.py ${LOADER}
COPY checks/qwen38_fp4_ple.py /opt/llm/checks/qwen38_fp4_ple.py
RUN chmod -R a+rX /opt/llm
USER 65534:65534
RUN HOME=/tmp python3 /opt/llm/checks/qwen38_fp4_checkpoint.py ${LOADER}
RUN HOME=/tmp python3 /opt/llm/checks/qwen38_fp4_ple.py ${LOADER}
USER root

LABEL org.opencontainers.image.source="https://github.com/Sumatoshi-tech/sparkplane" \
      org.opencontainers.image.revision="b12x-8244d1c44-pread-v2" \
      sparkplane.base-manifest="sha256:03a956544e9f9b4b7ba91690469fbe7848bdbef29a7d2e0f5788368467bb3595" \
      sparkplane.cuda-architecture="12.1a" \
      sparkplane.ple-transport="bounded-pread" \
      sparkplane.checkpoint-transport="bounded-pread" \
      sparkplane.checkpoint-self-test="qwen38_fp4_checkpoint.py"
