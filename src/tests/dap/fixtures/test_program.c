/*
 * Test program for DAP integration tests — C variant.
 *
 * Exercises: launch with stopOnEntry, line breakpoints, continue,
 * step over/into, stack trace, variable inspection, expression
 * evaluation.  Intended to be run with lldb-dap or gdb.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* --- data types for variable inspection --- */

struct counter {
    int value;
    const char *label;
};

/* --- recursive function for deeper stack traces --- */

long factorial(long n) {
    if (n <= 1) return 1;
    return n * factorial(n - 1);
}

/* --- loop with conditional — exercise conditional breakpoints --- */

void process_items(const int *items, size_t count, int *out) {
    for (size_t i = 0; i < count; i++) {
        int doubled = items[i] * 2;      /* conditional bp: items[i] > 10 */
        out[i] = doubled;
    }
}

/* --- nested calls for step_in / step_out --- */

int inner(int x) {
    int square = x * x;
    return square;
}

int middle(int x) {
    int y = x + 3;
    int z = inner(y);
    return z + 1;
}

int outer(void) {
    int result = middle(5);
    return result * 2;
}

/* --- main entry point --- */

int main(void) {
    /* basic types to inspect */
    int number = 42;
    double pi = 3.14159;
    char *text = "Hello, DAP!";
    int flag = 1;

    int items[] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 15, 20};
    size_t item_count = sizeof(items) / sizeof(items[0]);
    int doubled[20] = {0};

    struct counter c = {.value = 10, .label = "counter"};

    /* [bp-1] inspect locals */
    printf("number = %d\n", number);
    printf("pi = %.5f\n", pi);
    printf("text = %s\n", text);
    printf("flag = %d\n", flag);

    /* loop: step_over friendly */
    process_items(items, item_count, doubled);
    printf("doubled[0] = %d, doubled[last] = %d\n", doubled[0], doubled[item_count - 1]);

    /* [bp-2] after loop — try 'p doubled[3]' */

    /* recursion */
    long fact = factorial(5);
    printf("factorial(5) = %ld\n", fact);

    /* object mutation */
    c.value += 1;
    c.value += 1;
    printf("counter.value = %d\n", c.value);

    /* [bp-3] after counter ops — try 'p c.value' */

    /* nested calls */
    int outer_result = outer();
    printf("outer_result = %d\n", outer_result);

    /* [bp-4] near end — try 'p number + outer_result' */

    /* direct expression targets */
    int x = 10;
    int y = 20;
    int z = x + y;
    printf("z = %d\n", z);

    return 0;
}
