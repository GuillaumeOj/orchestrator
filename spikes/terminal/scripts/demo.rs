// Ligature and glyph test: => -> != === <= >= :: ||
fn main() {
    let xs = vec![1, 2, 3];
    if xs.len() >= 2 && xs[0] != 0 { println!("ok => {:?}", xs); }
}
