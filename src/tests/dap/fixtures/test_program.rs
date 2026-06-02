// Test program for DAP integration tests — Rust variant.
//
// Exercises: launch with stopOnEntry, line breakpoints, continue,
// step over/into, stack trace, variable inspection, expression
// evaluation.  Intended to be run with lldb-dap or gdb.

use std::collections::HashMap;

/// Simple struct for object inspection.
struct Counter {
    value: i32,
    label: &'static str,
}

impl Counter {
    fn new(start: i32) -> Self {
        Counter { value: start, label: "counter" }
    }

    fn increment(&mut self) -> i32 {
        self.value += 1;
        self.value
    }
}

/// Recursive function for deeper stack traces.
fn factorial(n: u64) -> u64 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

/// Process a vec — exercise iteration.
fn process_items(items: &[i32]) -> Vec<i32> {
    let mut results = Vec::new();
    for &item in items {
        let doubled = item * 2; // conditional bp: item > 10
        results.push(doubled);
    }
    results
}

/// Nested calls for step_in / step_out.
fn inner(x: i32) -> i32 {
    let square = x * x;
    square
}

fn middle(x: i32) -> i32 {
    let y = x + 3;
    let z = inner(y);
    z + 1
}

fn outer() -> i32 {
    let result = middle(5);
    result * 2
}

fn main() {
    // Basic types to inspect.
    let text = "Hello, DAP!";
    let number: i32 = 42;
    let pi: f64 = 3.14159;
    let flag = true;

    let items = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 15, 20];
    let mut mapping = HashMap::new();
    mapping.insert("key_a", 100);
    mapping.insert("key_b", 200);

    let mut counter = Counter::new(10);

    // [bp-1] inspect locals.
    println!("text = {text}");
    println!("number = {number}");
    println!("pi = {pi}");
    println!("flag = {flag}");
    println!("items len = {}", items.len());

    // Loop: step_over friendly.
    let doubled = process_items(&items);
    println!("doubled[0] = {}, doubled[last] = {}", doubled[0], doubled.last().unwrap());

    // [bp-2] after loop — try 'p doubled'.

    // Recursion.
    let fact = factorial(5);
    println!("factorial(5) = {fact}");

    // Object mutation.
    counter.increment();
    counter.increment();
    println!("counter.value = {}", counter.value);

    // [bp-3] after counter ops — try 'p counter.value'.

    // Nested calls.
    let outer_result = outer();
    println!("outer_result = {outer_result}");

    // [bp-4] near end.

    let x = 10i32;
    let y = 20i32;
    let z = x + y;
    println!("z = {z}");
}
