"""Exercise the actual B12X checkpoint reader with CPU CUDA/uring test doubles.

Only CUDA transport is simulated. File reads use real aligned O_DIRECT I/O;
the installed reader supplies all slot ownership, striding and conversion code.
"""

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


URING = r"""
#pragma once
#include <sys/uio.h>
struct io_uring { int unused; };
struct io_uring_sqe { uint64_t data; };
struct io_uring_cqe { uint64_t data; int res; };
static int uring_calls;
static inline int io_uring_queue_init(unsigned n, struct io_uring *r, unsigned f) {
  (void)n; (void)r; (void)f; uring_calls++; return -EPERM;
}
static inline void io_uring_queue_exit(struct io_uring *r) { (void)r; uring_calls++; }
static inline int io_uring_register_buffers(struct io_uring *r, const struct iovec *v, unsigned n) {
  (void)r; (void)v; (void)n; uring_calls++; return -EPERM;
}
static inline struct io_uring_sqe *io_uring_get_sqe(struct io_uring *r) { (void)r; uring_calls++; return NULL; }
static inline void io_uring_prep_read_fixed(struct io_uring_sqe *s, int f, void *b, unsigned n, int64_t o, int i) {
  (void)s; (void)f; (void)b; (void)n; (void)o; (void)i; uring_calls++;
}
static inline void io_uring_sqe_set_data64(struct io_uring_sqe *s, uint64_t d) { s->data=d; }
static inline int io_uring_submit(struct io_uring *r) { (void)r; uring_calls++; return -EPERM; }
static inline int io_uring_wait_cqe(struct io_uring *r, struct io_uring_cqe **c) { (void)r; (void)c; uring_calls++; return -EPERM; }
static inline uint64_t io_uring_cqe_get_data64(struct io_uring_cqe *c) { return c->data; }
static inline void io_uring_cqe_seen(struct io_uring *r, struct io_uring_cqe *c) { (void)r; (void)c; uring_calls++; }
"""

PREFIX = r"""
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>
#include <sys/stat.h>
#include <unistd.h>
#define IO_ALIGNMENT 4096
typedef struct { char message[1024]; } failure_t;
typedef void *cudaStream_t;
enum { cudaHostRegisterDefault=0, cudaStreamNonBlocking=1, cudaMemcpyHostToDevice=1 };
static atomic_int fail_copy;
static int pread_fault;
static bool cuda_ok(int code, const char *where, failure_t *f) {
  if (code && !f->message[0]) snprintf(f->message,sizeof(f->message),"%s: CUDA failure",where);
  return !code;
}
static void system_error(failure_t *f, const char *where) { snprintf(f->message,sizeof(f->message),"%s: %s",where,strerror(errno)); }
static int cudaGetDevice(int *d) { *d=0; return 0; }
static int cudaSetDevice(int d) { (void)d; return 0; }
static int cudaHostRegister(void *p,size_t n,int flags) { (void)p;(void)n;(void)flags;return 0; }
static int cudaHostUnregister(void *p) { (void)p;return 0; }
static int cudaStreamCreateWithFlags(cudaStream_t *s,int f) { (void)f;*s=(void*)1;return 0; }
static int cudaStreamDestroy(cudaStream_t s) { (void)s;return 0; }
static int cudaStreamSynchronize(cudaStream_t s) { (void)s;return 0; }
static int cudaMemcpyAsync(void *d,const void *s,size_t n,int k,cudaStream_t stream) {
  (void)k;(void)stream;if(atomic_load(&fail_copy))return 1;memcpy(d,s,n);return 0;
}
static int cudaMemcpy2DAsync(void *d,size_t dp,const void *s,size_t sp,size_t w,size_t h,int k,cudaStream_t stream) {
  (void)k;(void)stream;if(atomic_load(&fail_copy))return 1;
  for(size_t r=0;r<h;r++) { memcpy((char*)d+r*dp,(const char*)s+r*sp,w); }
  return 0;
}
static inline ssize_t checked_pread(int fd,void *b,size_t n,off_t o) {
  if(pread_fault==1){pread_fault=0;errno=EINTR;return -1;}
  if(pread_fault==2)return 0;
  if(pread_fault==3){errno=EIO;return -1;}
  return pread(fd,b,n,o);
}
"""

CASES = r"""
static bounce_ring_t *reader(void) {
  failure_t f={{0}};bounce_ring_t *r=bounce_create(0,8,&f);
  if(!r)fprintf(stderr,"reader initialization: %s\n",f.message);
  assert(r);assert(!r->ring_ready);return r;
}
int main(int argc,char **argv) {
  assert(argc==2);assert(setenv("B12X_CHECKPOINT_PREAD","1",1)==0);
  const size_t size=21*1024*1024;unsigned char *source=malloc(size);assert(source);
  uint32_t seed=7319;for(size_t i=0;i<size;i++){seed=seed*1664525u+1013904223u;source[i]=seed>>24;}
  char path[4096];assert(snprintf(path,sizeof(path),"%s/checkpoint.bin",argv[1])>0);
  int output=open(path,O_WRONLY|O_CREAT|O_EXCL,0600);assert(output>=0);
  size_t done=0;while(done<size){ssize_t n=write(output,source+done,size-done);assert(n>0);done+=(size_t)n;}assert(close(output)==0);
  int fd=open(path,O_RDONLY|O_DIRECT);assert(fd>=0);
  size_t length=20*1024*1024+79;unsigned char *dest=malloc(length+64);assert(dest);memset(dest,0xa5,length+64);
  bounce_ring_t *r=reader();failure_t f={{0}};
  read_job_t job={.fd=fd,.offset=113,.bytes=(int64_t)length,.destination=(char*)dest,.rows=1};
  pread_fault=1;assert(bounce_execute(r,&job,1,&f));assert(!f.message[0]);assert(memcmp(dest,source+113,length)==0);
  for(size_t i=length;i<length+64;i++)assert(dest[i]==0xa5);
  assert(r->reads>16 && r->submits==0);bounce_release(r);
  /* Strided rows retain untouched destination gaps. */
  memset(dest,0xa5,length+64);r=reader();memset(&f,0,sizeof(f));
  job=(read_job_t){.fd=fd,.offset=277,.bytes=32,.destination=(char*)dest,.rows=4096,.source_stride=80,.destination_stride=64};
  assert(bounce_execute(r,&job,1,&f));
  for(size_t row=0;row<4096;row++){assert(memcmp(dest+row*64,source+277+row*80,32)==0);for(size_t j=32;j<64;j++)assert(dest[row*64+j]==0xa5);}
  bounce_release(r);
  /* BF16 expansion preserves the exact source bits and destination pitch. */
  memset(dest,0xa5,length+64);r=reader();memset(&f,0,sizeof(f));
  job=(read_job_t){.fd=fd,.offset=512,.bytes=128,.destination=(char*)dest,.expand_bf16=true,.rows=512,.source_stride=256,.destination_stride=512};
  assert(bounce_execute(r,&job,1,&f));
  for(size_t row=0;row<512;row++)for(size_t col=0;col<64;col++){uint16_t b;uint32_t v;memcpy(&b,source+512+row*256+col*2,2);memcpy(&v,dest+row*512+col*4,4);assert(v==((uint32_t)b<<16));}
  bounce_release(r);
  r=reader();memset(&f,0,sizeof(f));memset(dest,0xa5,length+64);
  job=(read_job_t){.offset=(int64_t)(uintptr_t)source,.bytes=512,.destination=(char*)dest,.host_copy=true,.rows=1};
  assert(bounce_execute(r,&job,1,&f));assert(memcmp(dest,source,512)==0);bounce_release(r);
  /* Failed reads poison the executor and never publish stale slot bytes. */
  for(int fault=2;fault<=3;fault++){
    r=reader();memset(&f,0,sizeof(f));memset(dest,0xa5,length+64);pread_fault=fault;
    job=(read_job_t){.fd=fd,.offset=0,.bytes=512,.destination=(char*)dest,.rows=1};
    assert(!bounce_execute(r,&job,1,&f));assert(f.message[0] && r->poisoned);
    for(size_t i=0;i<512;i++)assert(dest[i]==0xa5);
    pread_fault=0;memset(&f,0,sizeof(f));assert(!bounce_execute(r,&job,1,&f));bounce_release(r);
  }
  r=reader();memset(&f,0,sizeof(f));atomic_store(&fail_copy,1);
  assert(!bounce_execute(r,&job,1,&f));assert(r->poisoned);bounce_release(r);atomic_store(&fail_copy,0);
  assert(uring_calls==0);
  /* The existing mode still fails closed when io_uring is unavailable. */
  assert(unsetenv("B12X_CHECKPOINT_PREAD")==0);memset(&f,0,sizeof(f));
  assert(!bounce_create(0,8,&f));assert(uring_calls==1 && f.message[0]);
  assert(close(fd)==0);free(dest);free(source);
  puts("{\"pread_checkpoint_cases\":8,\"passed\":true}");return 0;
}
"""


def main():
    root = Path(sys.argv[1])
    batch = (root / "_batch.c").read_text()
    job = batch[batch.index("typedef struct {"):batch.index("} read_job_t;") + len("} read_job_t;")]
    direct = (root / "_direct.c").read_text()
    validate = direct[direct.index("static bool validate_direct_range("):direct.index("static bool direct_read_range(")]
    compiler = shutil.which("cc") or shutil.which("gcc")
    if not compiler:
        raise SystemExit("C compiler required for checkpoint-reader checks")
    with tempfile.TemporaryDirectory(prefix="spark-fp4-reader-") as temporary:
        directory = Path(temporary)
        (directory / "liburing.h").write_text(URING)
        (directory / "_bounce.c").write_bytes((root / "_bounce.c").read_bytes())
        (directory / "check.c").write_text(PREFIX + job + validate + '\n#define pread checked_pread\n#include "_bounce.c"\n' + CASES)
        subprocess.run([compiler, "-std=c11", "-D_GNU_SOURCE", "-DB12X_HAVE_LIBURING=1", "-O2", "-Wall", "-Wextra", "-Werror", "-pthread", "-I", str(directory), str(directory / "check.c"), "-o", str(directory / "check")], check=True, timeout=60)
        subprocess.run([str(directory / "check"), str(directory)], check=True, timeout=30)


if __name__ == "__main__":
    main()
