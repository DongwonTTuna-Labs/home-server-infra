#define _GNU_SOURCE

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#ifndef AUTH_GUARD_PRELOAD
#define AUTH_GUARD_PRELOAD "/opt/codex-runner/libcodex-deny-auth.so"
#endif

static const char *auth_guard_path(void) {
  const char *path = getenv("CODEX_AUTH_GUARD_PATH");
  if (path && path[0]) {
    return path;
  }

  const char *codex_home = getenv("CODEX_HOME");
  if (!codex_home || !codex_home[0]) {
    codex_home = "/home/runner/.codex";
  }

  static char fallback[PATH_MAX];
  snprintf(fallback, sizeof(fallback), "%s/auth.json", codex_home);
  return fallback;
}

static bool has_suffix(const char *value, const char *suffix) {
  size_t value_len = strlen(value);
  size_t suffix_len = strlen(suffix);
  return value_len >= suffix_len &&
         strcmp(value + value_len - suffix_len, suffix) == 0;
}

static bool is_denied_path(const char *path) {
  if (!path || !path[0]) {
    return false;
  }

  const char *guard = auth_guard_path();
  char resolved_path[PATH_MAX];
  char resolved_guard[PATH_MAX];

  const char *path_to_check = path;
  if (realpath(path, resolved_path)) {
    path_to_check = resolved_path;
  }

  const char *guard_to_check = guard;
  if (realpath(guard, resolved_guard)) {
    guard_to_check = resolved_guard;
  }

  if (strcmp(path_to_check, guard_to_check) == 0) {
    return true;
  }

  return strstr(path_to_check, "/.codex/auth.json") != NULL ||
         has_suffix(path_to_check, "/auth.json");
}

static bool is_denied_at_path(int dirfd, const char *path) {
  if (!path || path[0] == '/') {
    return is_denied_path(path);
  }

  char absolute[PATH_MAX];
  if (dirfd == AT_FDCWD) {
    if (!getcwd(absolute, sizeof(absolute))) {
      return is_denied_path(path);
    }
  } else {
    char fd_path[64];
    snprintf(fd_path, sizeof(fd_path), "/proc/self/fd/%d", dirfd);
    ssize_t len = readlink(fd_path, absolute, sizeof(absolute) - 1);
    if (len < 0) {
      return is_denied_path(path);
    }
    absolute[len] = '\0';
  }

  size_t len = strlen(absolute);
  if (len + 1 + strlen(path) + 1 > sizeof(absolute)) {
    return is_denied_path(path);
  }
  absolute[len] = '/';
  strcpy(absolute + len + 1, path);
  return is_denied_path(absolute);
}

static void deny_auth(void) {
  errno = EACCES;
}

static const char *preload_path(void) {
  const char *path = getenv("CODEX_AUTH_GUARD_PRELOAD");
  return path && path[0] ? path : AUTH_GUARD_PRELOAD;
}

static bool preload_contains_self(const char *value, const char *self) {
  if (!value || !value[0]) {
    return false;
  }

  const char *cursor = value;
  size_t self_len = strlen(self);
  while (*cursor) {
    const char *end = strchr(cursor, ':');
    size_t len = end ? (size_t)(end - cursor) : strlen(cursor);
    if (len == self_len && strncmp(cursor, self, len) == 0) {
      return true;
    }
    if (!end) {
      break;
    }
    cursor = end + 1;
  }
  return false;
}

static char **env_with_preload(char *const envp[]) {
  const char *self = preload_path();
  const char *old_preload = NULL;
  size_t count = 0;

  if (envp) {
    for (; envp[count]; count++) {
      if (strncmp(envp[count], "LD_PRELOAD=", 11) == 0) {
        old_preload = envp[count] + 11;
      }
    }
  }

  if (preload_contains_self(old_preload, self)) {
    return (char **)envp;
  }

  char **next_env = calloc(count + 2, sizeof(char *));
  if (!next_env) {
    return (char **)envp;
  }

  size_t out = 0;
  for (size_t i = 0; i < count; i++) {
    if (strncmp(envp[i], "LD_PRELOAD=", 11) != 0) {
      next_env[out++] = envp[i];
    }
  }

  size_t needed = strlen("LD_PRELOAD=") + strlen(self) + 1;
  if (old_preload && old_preload[0]) {
    needed += 1 + strlen(old_preload);
  }

  char *preload = malloc(needed);
  if (!preload) {
    free(next_env);
    return (char **)envp;
  }

  if (old_preload && old_preload[0]) {
    snprintf(preload, needed, "LD_PRELOAD=%s:%s", self, old_preload);
  } else {
    snprintf(preload, needed, "LD_PRELOAD=%s", self);
  }

  next_env[out++] = preload;
  next_env[out] = NULL;
  return next_env;
}

static void free_env_if_needed(char **new_env, char *const old_env[]) {
  if (new_env == (char **)old_env) {
    return;
  }

  for (size_t i = 0; new_env[i]; i++) {
    if (strncmp(new_env[i], "LD_PRELOAD=", 11) == 0) {
      free(new_env[i]);
      break;
    }
  }
  free(new_env);
}

int open(const char *pathname, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }

  if (is_denied_path(pathname)) {
    deny_auth();
    return -1;
  }

  static int (*real_open)(const char *, int, ...) = NULL;
  if (!real_open) {
    real_open = dlsym(RTLD_NEXT, "open");
  }
  return (flags & O_CREAT) ? real_open(pathname, flags, mode)
                           : real_open(pathname, flags);
}

int open64(const char *pathname, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }

  if (is_denied_path(pathname)) {
    deny_auth();
    return -1;
  }

  static int (*real_open64)(const char *, int, ...) = NULL;
  if (!real_open64) {
    real_open64 = dlsym(RTLD_NEXT, "open64");
  }
  return (flags & O_CREAT) ? real_open64(pathname, flags, mode)
                           : real_open64(pathname, flags);
}

int openat(int dirfd, const char *pathname, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }

  if (is_denied_at_path(dirfd, pathname)) {
    deny_auth();
    return -1;
  }

  static int (*real_openat)(int, const char *, int, ...) = NULL;
  if (!real_openat) {
    real_openat = dlsym(RTLD_NEXT, "openat");
  }
  return (flags & O_CREAT) ? real_openat(dirfd, pathname, flags, mode)
                           : real_openat(dirfd, pathname, flags);
}

int openat64(int dirfd, const char *pathname, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }

  if (is_denied_at_path(dirfd, pathname)) {
    deny_auth();
    return -1;
  }

  static int (*real_openat64)(int, const char *, int, ...) = NULL;
  if (!real_openat64) {
    real_openat64 = dlsym(RTLD_NEXT, "openat64");
  }
  return (flags & O_CREAT) ? real_openat64(dirfd, pathname, flags, mode)
                           : real_openat64(dirfd, pathname, flags);
}

FILE *fopen(const char *pathname, const char *mode) {
  if (is_denied_path(pathname)) {
    deny_auth();
    return NULL;
  }

  static FILE *(*real_fopen)(const char *, const char *) = NULL;
  if (!real_fopen) {
    real_fopen = dlsym(RTLD_NEXT, "fopen");
  }
  return real_fopen(pathname, mode);
}

FILE *fopen64(const char *pathname, const char *mode) {
  if (is_denied_path(pathname)) {
    deny_auth();
    return NULL;
  }

  static FILE *(*real_fopen64)(const char *, const char *) = NULL;
  if (!real_fopen64) {
    real_fopen64 = dlsym(RTLD_NEXT, "fopen64");
  }
  return real_fopen64(pathname, mode);
}

int access(const char *pathname, int mode) {
  if (is_denied_path(pathname)) {
    deny_auth();
    return -1;
  }

  static int (*real_access)(const char *, int) = NULL;
  if (!real_access) {
    real_access = dlsym(RTLD_NEXT, "access");
  }
  return real_access(pathname, mode);
}

int faccessat(int dirfd, const char *pathname, int mode, int flags) {
  if (is_denied_at_path(dirfd, pathname)) {
    deny_auth();
    return -1;
  }

  static int (*real_faccessat)(int, const char *, int, int) = NULL;
  if (!real_faccessat) {
    real_faccessat = dlsym(RTLD_NEXT, "faccessat");
  }
  return real_faccessat(dirfd, pathname, mode, flags);
}

int execve(const char *filename, char *const argv[], char *const envp[]) {
  static int (*real_execve)(const char *, char *const[], char *const[]) = NULL;
  if (!real_execve) {
    real_execve = dlsym(RTLD_NEXT, "execve");
  }

  char **next_env = env_with_preload(envp);
  int result = real_execve(filename, argv, next_env);
  int saved_errno = errno;
  free_env_if_needed(next_env, envp);
  errno = saved_errno;
  return result;
}
