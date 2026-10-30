"""Add confined bounded direct pread waves to the pinned disk PLE reader."""
import hashlib
from pathlib import Path
import sys

root = Path(sys.argv[1])
path = root / '_ple_reader.c'
if hashlib.sha256(path.read_bytes()).hexdigest() != '36bcce65d3e131a02749a21105eda1771f80ca0e55686f84c576240fa783f585':
    raise SystemExit("unsupported B12X PLE source; refusing to patch")
source = path.read_text()
old, new = '  bool poisoned;\n', '  bool poisoned;\n  bool pread_mode;\n'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = '  reader->slots_count = (unsigned)depth;', '  const char* backend = getenv("B12X_DISK_BACKEND");\n  reader->pread_mode = backend && strcmp(backend, "pread") == 0;\n  /* A finite worker/64 KiB staging slot per concurrent direct read. */\n  reader->slots_count = reader->pread_mode && depth > 16 ? 16 : (unsigned)depth;'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = '  error = io_uring_queue_init(reader->slots_count, &reader->ring, 0);\n  if (error < 0) {\n    ple_release(reader);\n    return PyErr_Format(\n        PyExc_RuntimeError,\n        "io_uring initialization failed: %s; enable io_uring in the kernel/container policy",\n        strerror(-error));\n  }\n  reader->ring_ready = true;\n  reader->iovecs = calloc(reader->slots_count, sizeof(*reader->iovecs));\n  reader->free_slots = calloc(reader->slots_count, sizeof(*reader->free_slots));\n  if (!reader->iovecs || !reader->free_slots) {\n    ple_release(reader);\n    return PyErr_NoMemory();\n  }\n  for (unsigned i = 0; i < reader->slots_count; i++) {\n    reader->iovecs[i].iov_base = reader->slots[i].buffer;\n    reader->iovecs[i].iov_len = PLE_READ_MAX;\n  }\n  error = io_uring_register_buffers(&reader->ring, reader->iovecs, reader->slots_count);\n  if (error < 0) {\n    ple_release(reader);\n    return PyErr_Format(PyExc_RuntimeError,\n                        "io_uring buffer registration failed: %s; raise the locked-memory limit or "\n                        "reduce queue_depth",\n                        strerror(-error));\n  }\n', '  if (!reader->pread_mode) {\n    error = io_uring_queue_init(reader->slots_count, &reader->ring, 0);\n    if (error < 0) {\n      ple_release(reader);\n      return PyErr_Format(\n          PyExc_RuntimeError,\n          "io_uring initialization failed: %s; enable io_uring in the kernel/container policy",\n          strerror(-error));\n    }\n    reader->ring_ready = true;\n    reader->iovecs = calloc(reader->slots_count, sizeof(*reader->iovecs));\n    reader->free_slots = calloc(reader->slots_count, sizeof(*reader->free_slots));\n    if (!reader->iovecs || !reader->free_slots) {\n      ple_release(reader);\n      return PyErr_NoMemory();\n    }\n    for (unsigned i = 0; i < reader->slots_count; i++) {\n      reader->iovecs[i].iov_base = reader->slots[i].buffer;\n      reader->iovecs[i].iov_len = PLE_READ_MAX;\n    }\n    error = io_uring_register_buffers(&reader->ring, reader->iovecs, reader->slots_count);\n    if (error < 0) {\n      ple_release(reader);\n      return PyErr_Format(PyExc_RuntimeError,\n                          "io_uring buffer registration failed: %s; raise the locked-memory limit or "\n                          "reduce queue_depth",\n                          strerror(-error));\n    }\n  }\n'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = '  ple_reader_t* reader = owner;\n', '  ple_reader_t* reader = owner;\n  if (reader->pread_mode) return 0;\n'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = 'static void ple_uring_run(ple_reader_t* reader) {', '/* Each wave owns disjoint aligned slots. Join every issued read before scatter,\n * failure return, slot reuse or reader destruction; no detached I/O survives. */\ntypedef struct {\n  int fd;\n  char* buffer;\n  unsigned length;\n  int64_t offset;\n  int result;\n} ple_pread_job_t;\n\nstatic void* ple_pread_worker(void* opaque) {\n  ple_pread_job_t* job = opaque;\n  ssize_t result;\n  do {\n    result = pread(job->fd, job->buffer, job->length, job->offset);\n  } while (result < 0 && errno == EINTR);\n  job->result = result < 0 ? -errno : (int)result;\n  return NULL;\n}\n\nstatic void ple_pread_run(ple_reader_t* reader) {\n  pthread_t workers[16];\n  ple_pread_job_t reads[16];\n  for (size_t next = 0; next < reader->plan.job_count;) {\n    size_t left = reader->plan.job_count - next;\n    unsigned count = left < reader->slots_count ? (unsigned)left : reader->slots_count;\n    unsigned started = 0;\n    for (unsigned i = 0; i < count; i++) {\n      ple_job_t* job = &reader->plan.jobs[next + i];\n      reads[i] = (ple_pread_job_t){.fd = reader->plan.files[job->file].fd,\n                                 .buffer = reader->slots[i].buffer,\n                                 .length = job->length, .offset = job->offset};\n      if (count == 1) {\n        ple_pread_worker(&reads[i]);\n      } else {\n        int error = pthread_create(&workers[i], NULL, ple_pread_worker, &reads[i]);\n        if (error) {\n          ple_error(reader, "PLE pread worker", error);\n          reader->poisoned = true;\n          break;\n        }\n      }\n      started++;\n    }\n    for (unsigned i = 0; i < started; i++) {\n      if (count > 1) pthread_join(workers[i], NULL);\n    }\n    /* All reads have finished. A failed wave never publishes stale slot data. */\n    for (unsigned i = 0; i < started; i++) {\n      ple_job_t* job = &reader->plan.jobs[next + i];\n      if (reads[i].result < 0 || (unsigned)reads[i].result != job->expected) {\n        ple_error(reader, "PLE pread", reads[i].result < 0 ? -reads[i].result : EIO);\n        reader->poisoned = true;\n      }\n    }\n    if (reader->poisoned) return;\n    for (unsigned i = 0; i < started; i++)\n      if (!ple_scatter(reader, next + i, reads[i].buffer, reads[i].result)) {\n        reader->poisoned = true;\n        return;\n      }\n    next += started;\n  }\n}\n\nstatic void ple_uring_run(ple_reader_t* reader) {'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = '    if (planned && reader->plan.job_count) ple_uring_run(reader);', '    if (planned && reader->plan.job_count) {\n      if (reader->pread_mode) ple_pread_run(reader);\n      else ple_uring_run(reader);\n    }'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = 'PLE io_uring reader is unusable after a submission/completion failure', 'PLE reader is unusable after an I/O failure'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
path.write_text(source)
path = root / '../sequence/_shared/disk_table.py'
if hashlib.sha256(path.read_bytes()).hexdigest() != '601348d52d488057911b5a0b22d36f0e922d80cb956c2658901bee56e63e7c9a':
    raise SystemExit("unsupported B12X PLE source; refusing to patch")
source = path.read_text()
old, new = 'self._backend not in ("io_uring", "gds")', 'self._backend not in ("io_uring", "pread", "gds")'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = 'B12X_DISK_BACKEND must be io_uring or gds', 'B12X_DISK_BACKEND must be io_uring, pread or gds'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
old, new = 'self._backend == "io_uring"', 'self._backend in ("io_uring", "pread")'
if source.count(old) != 1:
    raise SystemExit("ambiguous PLE source anchor")
source = source.replace(old, new, 1)
path.write_text(source)
