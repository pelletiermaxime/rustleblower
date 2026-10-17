/* Exercise bounded captured-path validation without BPF or elevated privileges. */
#define main collector_program_main
#include "collector.c"
#undef main
#include <assert.h>

int main(int argc, char **argv)
{
    if (argc == 4 && strcmp(argv[1], "--connect-test") == 0) {
        if (!connect_output(argv[2], number(argv[3], 0, UINT_MAX - 1)))
            return 1;
        return puts("transport-fixture") < 0 || fflush(stdout) != 0;
    }
    char encoded[USAGE_PATH_SIZE * 2 - 1];
    struct usage_path captured = {.length = sizeof("/usr/bin/curl"), .value = "/usr/bin/curl"};
    assert(captured_executable(&captured, encoded));
    assert(strcmp(encoded, "2f7573722f62696e2f6375726c") == 0);
    for (int invalid = -1; invalid <= 1; ++invalid) {
        captured.length = invalid;
        assert(!captured_executable(&captured, encoded));
    }
    captured.length = USAGE_PATH_SIZE + 1;
    assert(!captured_executable(&captured, encoded));
    captured.length = sizeof("/usr/bin/curl") + 1;
    assert(!captured_executable(&captured, encoded));
    captured.length = sizeof("/usr/bin/curl");
    captured.value[0] = 'x';
    assert(!captured_executable(&captured, encoded));
    memset(captured.value, 'x', sizeof(captured.value));
    captured.value[0] = '/';
    captured.length = USAGE_PATH_SIZE;
    assert(!captured_executable(&captured, encoded));
    captured.value[USAGE_PATH_SIZE - 1] = 0;
    assert(captured_executable(&captured, encoded));
    assert(strlen(encoded) == (USAGE_PATH_SIZE - 1) * 2);
    puts("passed: captured path hex encoding; invalid, overlong, relative and unterminated path refusal");
    return 0;
}
