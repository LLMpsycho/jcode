#[inline(never)]
fn read_seed() -> i32 {
    6
}

#[inline(never)]
fn scale(seed: i32) -> i32 {
    seed * 7
}

#[inline(never)]
fn finalize(value: i32) -> i32 {
    value - 1
}

fn main() {
    let result = finalize(scale(read_seed()));
    println!("{result}");
}
