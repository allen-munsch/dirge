/*
 * Test program for DAP integration tests — C++ variant.
 *
 * Exercises: launch with stopOnEntry, line breakpoints, continue,
 * step over/into, stack trace, variable inspection, expression
 * evaluation with objects (std::vector, std::map).
 * Intended to be run with lldb-dap or gdb.
 */

#include <iostream>
#include <string>
#include <vector>
#include <map>
#include <numeric>

/* --- class for object inspection --- */

class Counter {
public:
    Counter(int start = 0) : value_(start), label_("counter") {}

    int increment() { return ++value_; }
    int value() const { return value_; }
    const char *label() const { return label_; }

private:
    int value_;
    const char *label_;
};

/* --- recursive function for deeper stack traces --- */

long factorial(long n) {
    if (n <= 1) return 1;
    return n * factorial(n - 1);
}

/* --- process a vector — exercise iteration --- */

std::vector<int> process_items(const std::vector<int> &items) {
    std::vector<int> results;
    for (auto item : items) {
        int doubled = item * 2;     /* conditional bp: item > 10 */
        results.push_back(doubled);
    }
    return results;
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

int outer() {
    int result = middle(5);
    return result * 2;
}

/* --- main entry point --- */

int main() {
    /* basic types to inspect */
    std::string text = "Hello, DAP!";
    int number = 42;
    double pi = 3.14159;
    bool flag = true;

    std::vector<int> items = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 15, 20};
    std::map<std::string, int> mapping = {{"key_a", 100}, {"key_b", 200}};
    Counter counter(10);

    /* [bp-1] inspect locals: text, number, pi, flag, items, mapping */
    std::cout << "text = " << text << std::endl;
    std::cout << "number = " << number << std::endl;
    std::cout << "pi = " << pi << std::endl;
    std::cout << "flag = " << (flag ? "true" : "false") << std::endl;

    /* loop: step_over friendly */
    auto doubled = process_items(items);
    std::cout << "doubled size = " << doubled.size() << std::endl;
    std::cout << "doubled[0] = " << doubled[0] << std::endl;

    /* [bp-2] after loop — try 'p doubled[0]' */

    /* recursion */
    long fact = factorial(5);
    std::cout << "factorial(5) = " << fact << std::endl;

    /* object mutation */
    counter.increment();
    counter.increment();
    std::cout << "counter.value = " << counter.value() << std::endl;

    /* [bp-3] after counter ops — try 'p counter.value_' */

    /* nested calls */
    int outer_result = outer();
    std::cout << "outer_result = " << outer_result << std::endl;

    /* [bp-4] near end */

    int x = 10, y = 20;
    int z = x + y;
    std::cout << "z = " << z << std::endl;

    return 0;
}
