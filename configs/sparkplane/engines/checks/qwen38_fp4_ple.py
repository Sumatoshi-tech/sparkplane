"""Compile and exercise the installed PLE reader and planner with real O_DIRECT I/O.

No GPU is needed: PLE scatters into host buffers. The Python capsule API, planner,
file validation, worker ownership and error handling are the actual shipped code.
"""
import array
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import sysconfig
import tempfile

PREFIX = r'''
#define _GNU_SOURCE
#define PY_SSIZE_T_CLEAN
#include <Python.h>
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
#include <liburing.h>
typedef struct { char message[512]; } failure_t;
static atomic_int fault, interrupted, active, peak, launches;
static int uring_calls;
static void system_error(failure_t* f,const char* op) {
  snprintf(f->message,sizeof(f->message),"%s: %s",op,strerror(errno));
}
static int denied_init(unsigned n,struct io_uring* r,unsigned f) {
  (void)n;(void)r;(void)f;uring_calls++;return -EPERM;
}
static inline ssize_t checked_pread(int fd,void* b,size_t n,off_t o) {
  int running=atomic_fetch_add(&active,1)+1;
  int old=atomic_load(&peak);
  while(running>old && !atomic_compare_exchange_weak(&peak,&old,running)) {}
  ssize_t result;
  int mode=atomic_load(&fault);
  if(mode==1 && !atomic_exchange(&interrupted,1)){errno=EINTR;result=-1;}
  else if(mode==2)result=0;
  else if(mode==3){errno=EIO;result=-1;}
  else result=pread(fd,b,n,o);
  atomic_fetch_sub(&active,1);return result;
}
static inline int checked_create(pthread_t* t,const pthread_attr_t* a,void*(*f)(void*),void* p) {
  if(atomic_load(&fault)==4 && atomic_fetch_add(&launches,1)==2)return EAGAIN;
  return pthread_create(t,a,f,p);
}
#define io_uring_queue_init denied_init
#define pread checked_pread
#define pthread_create checked_create
#include "_ple_reader.c"
static PyObject* set_fault(PyObject* self,PyObject* arg) {
  (void)self;long value=PyLong_AsLong(arg);if(PyErr_Occurred())return NULL;
  atomic_store(&fault,(int)value);atomic_store(&interrupted,0);atomic_store(&launches,0);Py_RETURN_NONE;
}
static PyObject* counters(PyObject* self,PyObject* args) {
  (void)self;(void)args;return Py_BuildValue("iii",uring_calls,atomic_load(&active),atomic_load(&peak));
}
static PyMethodDef methods[]={
 {"reader",py_ple_reader,METH_VARARGS,NULL},
 {"add",py_ple_reader_add,METH_VARARGS,NULL},
 {"run",py_ple_reader_run,METH_VARARGS,NULL},
 {"stats",py_ple_reader_stats,METH_O,NULL},
 {"fault",set_fault,METH_O,NULL},{"counters",counters,METH_NOARGS,NULL},{NULL,NULL,0,NULL}};
static PyModuleDef module={PyModuleDef_HEAD_INIT,.m_name="_spark_ple",.m_size=-1,.m_methods=methods};
PyMODINIT_FUNC PyInit__spark_ple(void){return PyModule_Create(&module);}
'''


def main():
    root = Path(sys.argv[1])
    compiler = shutil.which("cc") or shutil.which("gcc")
    if not compiler:
        raise SystemExit("C compiler required for PLE checks")
    with tempfile.TemporaryDirectory(prefix="spark-fp4-ple-") as temporary:
        directory = Path(temporary)
        for name in ("_ple_reader.c", "_row_plan.h"):
            shutil.copyfile(root / name, directory / name)
        (directory / "check.c").write_text(PREFIX)
        library = directory / ("_spark_ple" + sysconfig.get_config_var("EXT_SUFFIX"))
        subprocess.run([compiler, "-std=c11", "-DB12X_HAVE_LIBURING=1", "-O2", "-Wall", "-Wextra", "-Werror", "-shared", "-fPIC", "-pthread", "-I", sysconfig.get_path("include"), str(directory / "check.c"), "-luring", "-o", str(library)], check=True, timeout=60)
        spec = importlib.util.spec_from_file_location("_spark_ple", library)
        native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(native)
        os.environ["B12X_DISK_BACKEND"] = "pread"
        rows, width, scale_width = 512, 8193, 17
        weights = bytes((i * 131 + i // 251) % 256 for i in range(rows * width))
        scales = bytes((i * 47 + i // 61) % 256 for i in range(rows * scale_width))
        sources = []
        for shard in range(2):
            for is_scale, data, stride, offset in ((False, weights, width, 113), (True, scales, scale_width, 73)):
                path = directory / f"source-{shard}-{is_scale}"
                path.write_bytes(bytes(offset) + data[shard*256*stride:(shard+1)*256*stride])
                sources.append((shard, str(path), offset, is_scale))

        def reader():
            handle = native.reader(256, rows, 0, rows, width, scale_width, 128, 1024)
            for source in sources:
                native.add(handle, *source)
            assert native.stats(handle)["staging_bytes"] == 16 * 65536
            return handle

        ids = [((i*71) % rows) for i in range(100)] + [3, 3, -1, rows]

        def check(handle):
            out = bytearray([165]) * (len(ids)*width + 16)
            scale = bytearray([165]) * (len(ids)*scale_width + 16)
            native.run(handle, array.array("q", ids), out, scale, len(ids))
            for i, row in enumerate(ids):
                assert out[i*width:(i+1)*width] == (weights[row*width:(row+1)*width] if 0 <= row < rows else bytes(width))
                assert scale[i*scale_width:(i+1)*scale_width] == (scales[row*scale_width:(row+1)*scale_width] if 0 <= row < rows else bytes(scale_width))
            assert out[-16:] == bytes([165])*16 and scale[-16:] == bytes([165])*16
            assert native.stats(handle)["submit_calls"] == 0

        handle = reader()
        native.fault(1)
        check(handle)  # EINTR, unaligned sources, multiple waves, shards and planes.
        native.fault(0)
        with ThreadPoolExecutor(max_workers=2) as pool:
            list(pool.map(lambda _: check(handle), range(4)))
        # Failed waves must drain, publish no stale bytes, and reject reader reuse.
        for mode in (2, 3, 4):
            broken = reader()
            native.fault(mode)
            out = bytearray([165]) * (len(ids)*width)
            scale = bytearray([165]) * (len(ids)*scale_width)
            try:
                native.run(broken, array.array("q", ids[:-2]), out, scale, len(ids)-2)
                raise AssertionError("injected failure was accepted")
            except RuntimeError:
                pass
            assert out == bytes([165])*len(out)
            assert native.counters()[1] == 0
            native.fault(0)
            try:
                native.run(broken, array.array("q", [0]), out, scale, 1)
                raise AssertionError("poisoned reader reused")
            except RuntimeError:
                pass
        assert native.counters()[0] == 0 and native.counters()[2] <= 16
        os.environ["B12X_DISK_BACKEND"] = "io_uring"
        try:
            native.reader(256, rows, 0, rows, width, scale_width, 128, 16)
            raise AssertionError("denied io_uring accepted")
        except RuntimeError:
            pass
        assert native.counters()[0] == 1
        print(json.dumps({"pread_ple_cases": 9, "passed": True}))


if __name__ == "__main__":
    main()
