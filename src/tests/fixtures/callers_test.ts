function run(): void {
    console.log("run");
}

function runner(): void {
    run();
}

function running(): void {
    runner();
}

function prune(): void {
    // unrelated
}

// additional usage in expression
const x = run();
