fn main() {
    let capacity: u64 = 8;

    let mask = capacity - 1;

    (0..capacity).for_each(|x| println!("{:?}", x & mask));
}
