use rand::{rngs::SmallRng, Rng, SeedableRng};
use rust_pairwise_testing::Generator;
use std::{hint::black_box, io};

#[derive(Clone)]
pub struct FixedStringGenerator {
    string: String,
}

impl Generator for FixedStringGenerator {
    type Output = String;

    fn next_payload(&mut self) -> Self::Output {
        self.string.clone()
    }
}

pub struct RandomVec(SmallRng, usize);

impl RandomVec {
    pub fn new(size: usize) -> Self {
        Self(SmallRng::seed_from_u64(42), size)
    }
}

impl Generator for RandomVec {
    type Output = Vec<u32>;

    fn next_payload(&mut self) -> Self::Output {
        let RandomVec(rng, size) = self;
        let mut v = vec![0; *size];
        rng.fill(&mut v[..]);
        v
    }
}

#[derive(Clone)]
pub struct RandomStringGenerator {
    string: String,
    char_indicies: Vec<usize>,
    rng: SmallRng,
    length: usize,
}

impl RandomStringGenerator {
    pub fn new() -> io::Result<Self> {
        let string = std::fs::read_to_string("./input.txt")?;
        let char_indicies = string
            .char_indices()
            .map(|(idx, _)| idx)
            .collect::<Vec<_>>();
        let rng = SmallRng::from_entropy();
        Ok(Self {
            string,
            char_indicies,
            rng,
            length: 50000,
        })
    }
}
impl Generator for RandomStringGenerator {
    type Output = String;

    fn next_payload(&mut self) -> Self::Output {
        let start = self
            .rng
            .gen_range(0..self.char_indicies.len() - self.length);

        let from = self.char_indicies[start];
        let to = self.char_indicies[start + self.length];
        self.string[from..to].to_string()
    }
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
pub fn sum(n: usize) -> usize {
    let mut sum = 0;
    for i in 0..black_box(n) {
        sum += black_box(i);
    }
    sum
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
pub fn factorial(mut n: usize) -> usize {
    let mut result = 1usize;
    while n > 0 {
        result = result.wrapping_mul(black_box(n));
        n -= 1;
    }
    result
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
pub fn std(s: &String) -> usize {
    s.chars().count()
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
pub fn std_count(s: &String) -> usize {
    let mut l = 0;
    for _ in s.chars() {
        l += 1;
    }
    l
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
pub fn std_count_rev(s: &String) -> usize {
    let mut l = 0;
    for _ in s.chars().rev() {
        l += 1;
    }
    l
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
pub fn std_5000(s: &String) -> usize {
    s.chars().take(5000).count()
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
pub fn std_4925(s: &String) -> usize {
    s.chars().take(4925).count()
}
