/* mallinfo-shim — report glibc's own allocator accounting, once a second, to a file.
 *
 * **Why this exists.** `spec/audit/evidence/n117-preregistration.md`'s decision table turns on `F`
 * (free-but-retained bytes) against `R` (resident bytes in the allocator's region class). Nothing in
 * the tree can read `F`: a Rust heap profiler instruments the global allocator and so counts what was
 * *requested and freed*, never what glibc kept; `/proc` reports resident pages without saying whether
 * any allocator call still owns them. This shim is the one instrument that separates "retained" from
 * "live" — the two rows of the table that no other measurement can tell apart.
 *
 * It is deliberately outside the node binary: the crate graph is `#![forbid(unsafe_code)]`, so the
 * node cannot call `mallinfo2` itself, and `LD_PRELOAD` changes nothing about the binary under test —
 * which matters because every measurement here is against an artifact built before this file existed.
 *
 * Load it with `LD_PRELOAD=/contracts/mallinfo-shim.so`, and point it at a writable path with
 * `MALLINFO_OUT` (default `/var/lib/rnode/mallinfo.txt`). Build it against the *runtime* image's libc,
 * or at least a glibc that exports `mallinfo2` (≥ 2.33).
 *
 * Fields written, one line per second:
 *   hblkhd    bytes in mmap'd blocks          (part of `F`)
 *   uordblks  bytes in use                    (`U` — "live" from glibc's point of view)
 *   fordblks  bytes free in non-mmapped space (the other part of `F`)
 *   arena     total non-mmapped bytes
 *
 * Caveat, stated because it is a real perturbation: the shim itself allocates (the stdio buffer, and
 * whatever `fprintf` needs). It is a fixed per-process cost of a few KiB against measurements in the
 * hundreds of MiB, and it is the same in every arm — but it is not zero, and a reader is entitled to
 * know that the instrument consumes.
 */
#define _GNU_SOURCE
#include <malloc.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

static void *report_forever(void *unused) {
    (void)unused;
    const char *path = getenv("MALLINFO_OUT");
    if (path == NULL) {
        path = "/var/lib/rnode/mallinfo.txt";
    }
    FILE *out = fopen(path, "a");
    if (out == NULL) {
        return NULL;
    }
    for (;;) {
        struct mallinfo2 mi = mallinfo2();
        struct timespec now;
        clock_gettime(CLOCK_REALTIME, &now);
        fprintf(out, "%lld.%03ld hblkhd=%zu uordblks=%zu fordblks=%zu arena=%zu\n",
                (long long)now.tv_sec, now.tv_nsec / 1000000,
                mi.hblkhd, mi.uordblks, mi.fordblks, mi.arena);
        fflush(out);
        sleep(1);
    }
    return NULL;
}

__attribute__((constructor)) static void start_reporting(void) {
    pthread_t thread;
    if (pthread_create(&thread, NULL, report_forever, NULL) == 0) {
        pthread_detach(thread);
    }
}
