/* jemalloc-stats-shim — report jemalloc's own accounting, once a second, to a file.
 *
 * **Why this exists.** #117 turns on one question — is the memory that reaches the cgroup ceiling *live*
 * or *held*? — and until now no instrument in the tree could answer it. `mallinfo-shim.c` reads glibc's
 * accounting, and glibc is no longer this process's allocator (jemalloc replaced it in `29863d2e5`).
 * jemalloc's `stats_print` is readable only at process **exit**, and this node frees its world on the way
 * out: a node stopped cleanly at 3258 MiB of `anon` printed `Allocated: 2.9 MiB` — the state *after*
 * unwinding, not the state the measurement is about. `stats_interval` prints per-arena blocks at roughly
 * 150,000 lines per tick, which perturbs the run it is measuring.
 *
 * One number decides it. `stats.allocated` is what jemalloc believes the application currently holds;
 * `stats.resident` is what it has resident on the application's behalf. Read at the ceiling:
 *
 *   allocated ~ anon                    -> live.  A structure, and a heap profile can name it.
 *   allocated << anon, resident ~ anon  -> held.  Purge rate; allocator configuration is the fix space.
 *
 * **This needs `_rjem_mallctl` in the binary's dynamic symbol table.** jemalloc is linked statically, so
 * by default the symbol lives in `.symtab` only and `dlsym` cannot find it — measured: 175 dynamic
 * symbols, none of them jemalloc's. Build with
 * `RNODE_BUILD_RUSTFLAGS='-C link-arg=-Wl,--export-dynamic'`. That flag changes only the dynamic symbol
 * table, not allocation behaviour, and it is the one difference between this artifact and the shipped one;
 * the shim writes the resolved symbol into its header so an artifact cannot be mistaken for one taken
 * without it.
 *
 * Load with `LD_PRELOAD=/contracts/jemalloc-stats-shim.so`, output path from `JEMALLOC_STATS_OUT`
 * (default `/var/lib/rnode/jemalloc-stats.txt`).
 *
 * **A trap worth writing down:** `LD_PRELOAD` is inherited by every process the loader starts, so running
 * `LD_PRELOAD=... timeout 30 ./rnode` preloads this into `timeout` first, where jemalloc's symbols do not
 * exist. That produced a `symbol lookup error: undefined symbol: _rjem_mallctl` and an hour of chasing a
 * resolution failure that was not real. Run the binary directly, or preload only where it is wanted.
 *
 * Deliberately outside the node binary: the crate graph is `#![forbid(unsafe_code)]`, so the node cannot
 * call `mallctl` itself. Caveat, stated because it is real: the shim allocates a little (its stdio buffer)
 * and takes jemalloc's stats lock once a second. It is a fixed, tiny, per-process cost, identical in
 * every arm — but it is not zero.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <pthread.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef int (*mallctl_fn)(const char *name, void *oldp, size_t *oldlenp, void *newp, size_t newlen);

static mallctl_fn ctl = NULL;
static const char *ctl_symbol = "(unresolved)";

static void resolve(void) {
    /* `tikv-jemalloc-sys` builds jemalloc with the `_rjem_` symbol prefix; plain `mallctl` resolves if a
     * build ever stops prefixing. Both are tried, and which one resolved is recorded. */
    ctl = (mallctl_fn)dlsym(RTLD_DEFAULT, "_rjem_mallctl");
    if (ctl != NULL) {
        ctl_symbol = "_rjem_mallctl";
        return;
    }
    ctl = (mallctl_fn)dlsym(RTLD_DEFAULT, "mallctl");
    if (ctl != NULL) {
        ctl_symbol = "mallctl";
    }
}

/* **The epoch must be advanced before every read, or the values are frozen.** jemalloc's merged
 * `stats.*` are cached and refreshed only when the epoch is bumped; reading them without a bump returns
 * the value from the first read, forever. Measured the hard way: a series of 319 one-second samples was
 * byte-identical — 78232 allocated, 6492160 resident, 14770176 mapped, every line — while the cgroup's
 * `anon` for the same process climbed past 4 GiB. That is an instrument reporting a still photograph and
 * looking exactly like a finding. */
static void advance_epoch(void) {
    size_t epoch = 1;
    if (ctl != NULL) {
        ctl("epoch", NULL, NULL, &epoch, sizeof(epoch));
    }
}

static long long read_stat(const char *name) {
    size_t v = 0;
    size_t sz = sizeof(v);
    if (ctl == NULL || ctl(name, &v, &sz, NULL, 0) != 0) {
        return -1;
    }
    return (long long)v;
}

/* Options must be read into a buffer of the option's *native* type: jemalloc writes the value and then
 * returns EINVAL if the length supplied did not match — so reading a `bool` into a `size_t` yields the
 * right byte and a non-zero return, which a `!= 0` check turns into a confident -1. Measured: every
 * `opt.*` field in this header read as -1 until the types below were fixed. */
static int opt_bool(const char *name) {
    unsigned char v = 0;
    size_t sz = sizeof(v);
    if (ctl == NULL || ctl(name, &v, &sz, NULL, 0) != 0) {
        return -1;
    }
    return v;
}

static long long opt_u32(const char *name) {
    unsigned int v = 0;
    size_t sz = sizeof(v);
    if (ctl == NULL || ctl(name, &v, &sz, NULL, 0) != 0) {
        return -1;
    }
    return (long long)v;
}

static long long opt_ssize(const char *name) {
    long long v = -1;
    size_t sz = sizeof(v);
    if (ctl == NULL || ctl(name, &v, &sz, NULL, 0) != 0) {
        return -1;
    }
    return v;
}

static void write_header(FILE *out) {
    /* The version is reported only if the call succeeds; "(unavailable)" otherwise. An artifact header
     * that misreports what it measured is the defect the register carries as C176, and the first version
     * of this file printed an uninitialised buffer there. */
    char version[64];
    const char *ver = "(unavailable)";
    memset(version, 0, sizeof(version));
    if (ctl != NULL) {
        size_t sz = sizeof(version);
        if (ctl("version", version, &sz, NULL, 0) == 0 && version[0] != '\0') {
            ver = version;
        }
    }
    fprintf(out,
            "# symbol=%s jemalloc=%s\n"
            "# the options the process is actually running, read back out of jemalloc rather than assumed:\n"
            "#   opt.retain=%d opt.background_thread=%d opt.dirty_decay_ms=%lld opt.muzzy_decay_ms=%lld "
            "opt.narenas=%lld arenas.narenas=%lld\n"
            "#   _RJEM_MALLOC_CONF=%s\n"
            "# columns: utc allocated_bytes active_bytes resident_bytes mapped_bytes retained_bytes metadata_bytes\n",
            ctl_symbol, ver,
            opt_bool("opt.retain"), opt_bool("opt.background_thread"),
            opt_ssize("opt.dirty_decay_ms"), opt_ssize("opt.muzzy_decay_ms"),
            opt_u32("opt.narenas"), opt_u32("arenas.narenas"),
            getenv("_RJEM_MALLOC_CONF") ? getenv("_RJEM_MALLOC_CONF") : "(unset)");
    fflush(out);
}

static void *report_forever(void *unused) {
    (void)unused;
    const char *path = getenv("JEMALLOC_STATS_OUT");
    if (path == NULL) {
        path = "/var/lib/rnode/jemalloc-stats.txt";
    }
    FILE *out = fopen(path, "a");
    if (out == NULL) {
        return NULL;
    }
    resolve();
    write_header(out);
    if (ctl == NULL) {
        fprintf(out, "# UNRESOLVED: no mallctl in the dynamic symbol table. This file contains no data.\n");
        fflush(out);
        return NULL;
    }
    for (;;) {
        struct timespec now;
        advance_epoch();
        clock_gettime(CLOCK_REALTIME, &now);
        fprintf(out, "%lld.%03ld %lld %lld %lld %lld %lld %lld\n",
                (long long)now.tv_sec, now.tv_nsec / 1000000,
                read_stat("stats.allocated"), read_stat("stats.active"),
                read_stat("stats.resident"), read_stat("stats.mapped"),
                read_stat("stats.retained"), read_stat("stats.metadata"));
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
