"""Add an explicit bounded pread transport to the pinned B12X bounce reader."""
import hashlib
from pathlib import Path
import sys

path = Path(sys.argv[1]) / '_bounce.c'
original = path.read_text()
if hashlib.sha256(path.read_bytes()).hexdigest() != '45235c9008d3b2c775dac65bdd38116cd266db881470ea8bdbd9e74964d9d3db':
    raise SystemExit('unsupported B12X checkpoint reader; refusing to patch')

def replace(old, new):
    global original
    if original.count(old) != 1:
        raise SystemExit('checkpoint reader patch anchor is ambiguous')
    original = original.replace(old, new, 1)

replace('  bool registered;\n', '  bool registered;\n  bool pread_mode;\n')
replace('  ring->depth = depth;\n', '''  ring->depth = depth;
  const char* pread_mode = getenv("B12X_CHECKPOINT_PREAD");
  ring->pread_mode = pread_mode && strcmp(pread_mode, "1") == 0;
''')
start = original.index('  error = io_uring_queue_init(BOUNCE_SLOTS, &ring->ring, 0);')
end = original.index('  error = pthread_create(&ring->copier, NULL, bounce_copy_worker, ring);', start)
block = original[start:end]
replace(block, '''  if (ring->pread_mode) {
    for (unsigned i = 0; i < BOUNCE_SLOTS; i++) ring->free_slots[ring->free_count++] = i;
  } else {
''' + ''.join('  '+line+'\n' for line in block.splitlines()) + '  }\n')
start = original.index('      struct io_uring_sqe* sqe = io_uring_get_sqe(&ring->ring);')
end = original.index('      within += slot->job.bytes;', start)
block = original[start:end]
replace(block, '''      if (ring->pread_mode) {
        /* The same fixed slot stays owned until its CUDA copy completes. */
        ssize_t received;
        do {
          received = pread(job->fd, ring->buffers + (size_t)index * BOUNCE_SLOT_BYTES,
                           length, aligned);
        } while (received < 0 && errno == EINTR);
        ring->reads++;
        if (received > 0) ring->physical_bytes += received;
        if (received < 0 || (size_t)received < slot->required) {
          snprintf(failure->message, sizeof(failure->message), "checkpoint pread: %s",
                   received < 0 ? strerror(errno) : "short O_DIRECT read");
          pthread_mutex_lock(&ring->mutex);
          ring->free_slots[ring->free_count++] = index;
          pthread_mutex_unlock(&ring->mutex);
          ring->poisoned = true;
          break;
        }
        if (!ring->max_inflight) ring->max_inflight = 1;
        pthread_mutex_lock(&ring->mutex);
        ring->ready_slots[ring->tail++ % BOUNCE_SLOTS] = index;
        ring->ready_count++;
        ring->copying++;
        pthread_cond_signal(&ring->ready);
        pthread_mutex_unlock(&ring->mutex);
      } else {
''' + ''.join('  '+line+'\n' for line in block.splitlines()) + '      }\n')
replace('"io_uring bounce executor is unusable after a submission/completion failure"',
        '"checkpoint bounce executor is unusable after a read/copy failure"')
replace('''  if (ring->poisoned) {
    /* Drain submitted reads above; never submit SQEs left by a failed submit. */''',
        '''  if (ring->poisoned && ring->ring_ready) {
    /* Drain submitted reads above; never submit SQEs left by a failed submit. */''')
path.write_text(original)
