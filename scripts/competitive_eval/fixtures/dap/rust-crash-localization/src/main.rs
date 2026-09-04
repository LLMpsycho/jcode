#[inline(never)]
fn select_label<'a>(requested_slot: usize, labels: &'a [&'a str]) -> &'a str {
    let storage_slot = requested_slot + 1;
    labels[storage_slot]
}

fn main() {
    let labels = ["alpha", "beta", "gamma"];
    println!("{}", select_label(2, &labels));
}
