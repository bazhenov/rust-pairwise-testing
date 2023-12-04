#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md"))]

use num_traits::ToPrimitive;
use std::{
    any::type_name,
    cell::RefCell,
    cmp::Ordering,
    hint::black_box,
    io,
    ops::{Add, Div, RangeInclusive},
    rc::Rc,
    str::Utf8Error,
    time::Duration,
};
use thiserror::Error;
use timer::{ActiveTimer, Timer};

pub mod cli;
pub mod dylib;
pub mod generators;
#[cfg(target_os = "linux")]
pub mod linux;

const NS_TO_MS: usize = 1_000_000;

#[derive(Debug, Error)]
pub enum Error {
    #[error("No measurements given")]
    NoMeasurements,

    #[error("Invalid string pointer from FFI")]
    InvalidFFIString(Utf8Error),

    #[error("Spi::self() was already called")]
    SpiSelfWasMoved,

    #[error("Unable to load library symbol")]
    UnableToLoadSymbol(#[source] libloading::Error),

    #[error("IO Error")]
    IOError(#[from] io::Error),
}

/// Registers benchmark in the system
///
/// Macros accepts a list of functions that produce any [`IntoBenchmarks`] type. All of the benchmarks
/// created by those functions are registered in the harness.
///
/// ## Example
/// ```rust
/// use std::time::Instant;
/// use tango_bench::{benchmark_fn, IntoBenchmarks, tango_benchmarks};
///
/// fn time_benchmarks() -> impl IntoBenchmarks {
///     [benchmark_fn("current_time", || Instant::now())]
/// }
///
/// tango_benchmarks!(time_benchmarks());
/// ```
#[macro_export]
macro_rules! tango_benchmarks {
    ($($func_expr:expr),+) => {
        #[no_mangle]
        pub fn __tango_create_benchmarks() -> Vec<Box<dyn $crate::MeasureTarget>> {
            let mut benchmarks = vec![];
            $(benchmarks.extend($crate::IntoBenchmarks::into_benchmarks($func_expr));)*
            benchmarks
        }
    };
}

/// Main entrypoint for benchmarks
///
/// This macro generate `main()` function for the benchmark harness. Can be used in a form with providing
/// measurement settings:
/// ```rust
/// use tango_bench::{tango_main, MeasurementSettings};
///
/// tango_main!(MeasurementSettings {
///     samples_per_haystack: 1000,
///     min_iterations_per_sample: 10,
///     max_iterations_per_sample: 10_000,
///     ..Default::default()
/// });
/// ```
#[macro_export]
macro_rules! tango_main {
    ($settings:expr) => {
        fn main() -> $crate::cli::Result<std::process::ExitCode> {
            $crate::cli::run($settings)
        }
    };
    () => {
        tango_main! {$crate::MeasurementSettings::default()}
    };
}

pub fn benchmark_fn<O, F: Fn() -> O + 'static>(
    name: &'static str,
    func: F,
) -> Box<dyn MeasureTarget> {
    assert!(!name.is_empty());
    Box::new(SimpleFunc { name, func })
}

pub trait MeasureTarget {
    /// Measures the performance if the function
    ///
    /// Returns the cumulative execution time (all iterations) with nanoseconds precision,
    /// but not necessarily accuracy. Usually this time is get by `clock_gettime()` call or some other
    /// platform-specific system call.
    ///
    /// This method should use the same arguments for measuring the test function unless [`next_haystack()`]
    /// method is called. Only then new set of input arguments should be generated. Although it is allowed
    /// to call this method without first calling [`next_haystack()`]. In which case first haystack should be
    /// generated automatically.
    ///
    /// [`next_haystack()`]: Self::next_haystack()
    fn measure(&mut self, iterations: usize) -> u64;

    /// Estimates the number of iterations achievable within given time.
    ///
    /// Time span is given in milliseconds (`time_ms`). Estimate can be an approximation and it is important
    /// for implementation to be fast (in the order of 10 ms).
    /// If possible the same input arguments should be used when building the estimate.
    /// If the single call of a function is longer than provided timespan the implementation should return 0.
    fn estimate_iterations(&mut self, time_ms: u32) -> usize;

    /// Generates next haystack for the measurement
    ///
    /// Calling this method should update internal haystack used for measurement. Returns `true` if update happend,
    /// `false` if implementation doesn't support haystack generation.
    /// Haystack/Needle distinction is described in [`Generator`] trait.
    fn next_haystack(&mut self) -> bool;

    fn reset(&mut self, seed: u64);

    /// Name of the benchmark
    fn name(&self) -> &str;
}

struct SimpleFunc<F> {
    name: &'static str,
    func: F,
}

impl<O, F: Fn() -> O> MeasureTarget for SimpleFunc<F> {
    fn measure(&mut self, iterations: usize) -> u64 {
        let mut result = Vec::with_capacity(iterations);
        let start = ActiveTimer::start();
        for _ in 0..iterations {
            result.push(black_box((self.func)()));
        }
        let time = ActiveTimer::stop(start);
        drop(result);
        time
    }

    fn estimate_iterations(&mut self, time_ms: u32) -> usize {
        let median = median_execution_time(self, 11) as usize;
        time_ms as usize * NS_TO_MS / median
    }

    fn next_haystack(&mut self) -> bool {
        false
    }

    fn name(&self) -> &str {
        self.name
    }

    fn reset(&mut self, _: u64) {}
}

/// Implementation of a [`MeasureTarget`] which uses [`Generator`] to generates a new payload for a function
/// each new sample.
pub struct GenFunc<F, G: Generator> {
    f: Rc<RefCell<F>>,
    g: Rc<RefCell<G>>,
    haystack: Option<G::Haystack>,
    name: String,
}

impl<F, O, G> GenFunc<F, G>
where
    G: Generator,
    F: Fn(&G::Haystack, &G::Needle) -> O,
{
    pub fn new(name: &str, f: F, g: G) -> Self {
        let f = Rc::new(RefCell::new(f));
        let g = Rc::new(RefCell::new(g));
        Self::from_ref_cell(name, f, g)
    }

    fn from_ref_cell(name: &str, f: Rc<RefCell<F>>, g: Rc<RefCell<G>>) -> Self {
        Self {
            name: format!("{}/{}", name, g.borrow().name()),
            haystack: None,
            f,
            g,
        }
    }
}

impl<F, O, G> MeasureTarget for GenFunc<F, G>
where
    G: Generator,
    F: Fn(&G::Haystack, &G::Needle) -> O,
{
    fn measure(&mut self, iterations: usize) -> u64 {
        let mut g = self.g.borrow_mut();
        let haystack = &*self.haystack.get_or_insert_with(|| g.next_haystack());

        let f = self.f.borrow_mut();
        let mut result = Vec::with_capacity(iterations);
        let start = ActiveTimer::start();
        for _ in 0..iterations {
            let needle = g.next_needle(haystack);
            result.push(black_box((f)(haystack, &needle)));
        }
        let time = ActiveTimer::stop(start);
        drop(result);
        time
    }

    fn estimate_iterations(&mut self, time_ms: u32) -> usize {
        // Here we relying on the fact that measure() is not generating a new haystack
        // without a call to next_haystack()
        let measurements = (0..11).map(|_| self.measure(1)).collect::<Vec<_>>();
        (time_ms as usize * NS_TO_MS) / median(measurements) as usize
    }

    fn next_haystack(&mut self) -> bool {
        self.haystack = Some(self.g.borrow_mut().next_haystack());
        true
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn reset(&mut self, seed: u64) {
        self.g.borrow_mut().reset(seed)
    }
}

/// Matrix of functions is used to perform benchmark with different generator strategies
///
/// It is a common task to benchmark function with different payload size and/or different structure of the payload.
/// `BenchmarkMatrix` creates a new [`MeasureTarget`] for each unique combination of [`Generator`]
/// and tested function.
///
/// # Example
/// ```rust
/// use tango_bench::{generators::RandomVec, BenchmarkMatrix, IntoBenchmarks};
///
/// fn sum_positive(haystack: &Vec<u32>, _: &()) -> u32 {
///     haystack.iter().copied().filter(|v| *v > 0).sum()
/// }
///
/// fn sorting_benchmarks() -> impl IntoBenchmarks {
///     BenchmarkMatrix::with_params([100, 1_000, 10_000], RandomVec::new)
///         .add_function("sum_positive", sum_positive)
/// }
/// ```
pub struct BenchmarkMatrix<G> {
    generators: Vec<Rc<RefCell<G>>>,
    functions: Vec<Box<dyn MeasureTarget>>,
}

impl<G: Generator> BenchmarkMatrix<G> {
    pub fn new(generator: G) -> Self {
        let generator = Rc::new(RefCell::new(generator));
        Self {
            generators: vec![generator],
            functions: vec![],
        }
    }

    /// New matrix with generator created for a given set of parameters
    pub fn with_params<P>(params: impl IntoIterator<Item = P>, generator: impl Fn(P) -> G) -> Self {
        let generators: Vec<_> = params
            .into_iter()
            .map(generator)
            .map(RefCell::new)
            .map(Rc::new)
            .collect();
        Self {
            generators,
            functions: vec![],
        }
    }

    pub fn add_generators_with_params<P>(
        mut self,
        params: impl IntoIterator<Item = P>,
        generator: impl Fn(P) -> G,
    ) -> Self {
        let generators = params
            .into_iter()
            .map(generator)
            .map(RefCell::new)
            .map(Rc::new);
        self.generators.extend(generators);
        self
    }

    pub fn add_function<F, O>(mut self, name: &str, f: F) -> Self
    where
        G: 'static,
        F: Fn(&G::Haystack, &G::Needle) -> O + 'static,
    {
        let f = Rc::new(RefCell::new(f));
        self.generators
            .iter()
            .map(Rc::clone)
            .map(|g| GenFunc::from_ref_cell(name, Rc::clone(&f), g))
            .map(Box::new)
            .for_each(|f| self.functions.push(f));
        self
    }
}

impl<G> IntoBenchmarks for BenchmarkMatrix<G> {
    fn into_benchmarks(self) -> Vec<Box<dyn MeasureTarget>> {
        assert!(!self.functions.is_empty(), "No functions was given");
        self.functions
    }
}

pub trait IntoBenchmarks {
    fn into_benchmarks(self) -> Vec<Box<dyn MeasureTarget>>;
}

impl<const N: usize> IntoBenchmarks for [Box<dyn MeasureTarget>; N] {
    fn into_benchmarks(self) -> Vec<Box<dyn MeasureTarget>> {
        self.into_iter().collect()
    }
}

impl IntoBenchmarks for Vec<Box<dyn MeasureTarget>> {
    fn into_benchmarks(self) -> Vec<Box<dyn MeasureTarget>> {
        self
    }
}

/// Generates the payload for the benchmarking functions
///
/// One of the most important parts of the benchmarking process is generating the payload to test the algorithm. This /// is what this trait is doing. Test function registered in the system can accepts two arguments:
/// - *haystack* - usually the data structure we're testing the algorithm on
/// - *needle* - the supplementary used to test the algorithm.
///
/// ## Haystack
/// Haystack is typically some sort of a collection that is used in benchmarking. It can be quite large and
/// expensive to generate, because it is generated once per sample or less. The frequency of haystack generation
/// is controlled by [`MeasurementSettings::samples_per_haystack`].
///
/// ## Needle
/// Needle is usually some type of query that is presented to the algorithm. In case of searching algorithm it
/// can be value we search in the collection.
///
/// Important distinction between haystack and needle is that haystack generation is not included in timing while
/// needle generation is a part of measurement loop. Therefore needle generation should be relativley lightweight.
///
/// Sometimes haystack generation might be so expensive that it makes sense to leave haystack fixed and provide
/// randomness by generating different needles. For example, instead of generating new random `Vec<T>` for each sample
/// it might be more practical to generate a single `Vec` and a new `Range<usize>` as a haystack at each iteration.
///
/// It might be the case that the algorithm being tested is not using both type of values.
/// In this case corresponding value type should unit type – `()`.
/// Depending on the type of algorithm you might not need to generate both of them. Here are some examples:
///
/// | Algorithm | Haystack | Needle |
/// |----------|----------|--------|
/// | Searching in a string | String | substrung to search for and/or range to search over |
/// | Searching in a collection | Collection | Value to search for and/or range to search over |
/// | Soring | Collection | – |
/// | Numerical computation: factorial, DP problems, etc. | – | Input parameters |
///
/// Tango orchestrates the generating of haystack and needle and guarantees that both benchmarking
/// functions are called with the same input parameters. Therefore performance difference is predictable.
pub trait Generator {
    type Haystack;
    type Needle;

    /// Generates next random haystack for the benchmark
    ///
    /// All iterations within sample are using the same haystack. Haystack are changed only between samples
    /// (see. [`MeasureTarget::next_haystack()`]).
    fn next_haystack(&mut self) -> Self::Haystack;

    /// Generates next random needle for the benchmark
    ///
    /// This method should be relatively lightweight, because the execution time of this method is included
    /// in reported by the benchmark time. Implementation are given haystack generated by
    /// [`Self::next_haystack()`] which will be used for benchmark execution.
    fn next_needle(&mut self, haystack: &Self::Haystack) -> Self::Needle;

    /// Resets internal RNG-state of this generator with given seed
    ///
    /// For benchmarks to be predictable the harness periodically synchronize the RNG state of all the generators.
    /// If applicable, implementations should set internal RNG state with the value derived from given `seed`.
    /// Implementation are free to transform seed value in any meaningfull way (like taking only lower 32 bits)
    /// as long as this transformation is deterministic.
    fn reset(&mut self, seed: u64);

    /// Name of generator
    fn name(&self) -> &str {
        let name = type_name::<Self>();
        if let Some(idx) = name.rfind("::") {
            // it's safe to operate on byte offsets here because ':' symbols is 1-byte ascii
            &name[idx + 2..]
        } else {
            name
        }
    }
}

pub trait Reporter {
    fn on_complete(&mut self, _results: &RunResult) {}
}

/// Describes basic settings for the benchmarking process
///
/// This structure is passed to [`cli::run()`].
///
/// Should be created only with overriding needed properties, like so:
/// ```rust
/// use tango_bench::MeasurementSettings;
///
/// let settings = MeasurementSettings {
///     max_samples: 10_000,
///     ..Default::default()
/// };
/// ```
#[derive(Clone, Copy, Debug)]
pub struct MeasurementSettings {
    pub max_samples: usize,
    pub max_duration: Duration,
    pub outlier_detection_enabled: bool,

    /// The number of samples per one generated haystack
    pub samples_per_haystack: usize,

    /// Minimum number of iterations in a sample for each of 2 tested functions
    pub min_iterations_per_sample: usize,

    /// The number of iterations in a sample for each of 2 tested functions
    pub max_iterations_per_sample: usize,
}

pub const DEFAULT_SETTINGS: MeasurementSettings = MeasurementSettings {
    max_samples: 1_000_000,
    max_duration: Duration::from_millis(100),
    outlier_detection_enabled: true,
    samples_per_haystack: 1,
    min_iterations_per_sample: 1,
    max_iterations_per_sample: 5000,
};

impl Default for MeasurementSettings {
    fn default() -> Self {
        DEFAULT_SETTINGS
    }
}

pub fn calculate_run_result<N: Into<String>>(
    name: N,
    mut baseline: Vec<u64>,
    mut candidate: Vec<u64>,
    iterations_per_sample: usize,
    filter_outliers: bool,
) -> Result<RunResult, Error> {
    assert!(baseline.len() == candidate.len());

    let mut diff = candidate
        .iter()
        .copied()
        .zip(baseline.iter().copied())
        // need to convert both of measurement to i64 because difference can be negative
        .map(|(c, b)| (c as i64, b as i64))
        .map(|(c, b)| (c - b) / iterations_per_sample as i64)
        .collect::<Vec<i64>>();

    let n = diff.len();

    // Normalizing measurements
    for v in baseline.iter_mut() {
        *v /= iterations_per_sample as u64;
    }
    for v in candidate.iter_mut() {
        *v /= iterations_per_sample as u64;
    }

    // Calculating measurements range. All measurements outside this interval concidered outliers
    let range = if filter_outliers {
        iqr_variance_thresholds(diff.to_vec())
    } else {
        None
    };

    // Cleaning measurements from outliers if needed
    if let Some(range) = range {
        // We filtering outliers to build statistical Summary and the order of elements in arrays
        // doesn't matter, therefore swap_remove() is used. But we need to make sure that all arrays
        // has the same length
        assert_eq!(diff.len(), baseline.len());
        assert_eq!(diff.len(), candidate.len());

        let mut i = 0;
        while i < diff.len() {
            if range.contains(&diff[i]) {
                i += 1;
            } else {
                diff.swap_remove(i);
                baseline.swap_remove(i);
                candidate.swap_remove(i);
            }
        }
    };

    let diff = Summary::from(&diff).ok_or(Error::NoMeasurements)?;
    let baseline = Summary::from(&baseline).ok_or(Error::NoMeasurements)?;
    let candidate = Summary::from(&candidate).ok_or(Error::NoMeasurements)?;

    let std_dev = diff.variance.sqrt();
    let std_err = std_dev / (diff.n as f64).sqrt();
    let z_score = diff.mean / std_err;

    Ok(RunResult {
        baseline,
        candidate,
        diff,
        name: name.into(),
        // significant result is far away from 0 and have more than 0.5%
        // base/candidate difference
        // z_score = 2.6 corresponds to 99% significance level
        significant: z_score.abs() >= 2.6 && (diff.mean / candidate.mean).abs() > 0.005,
        outliers: n - diff.n,
    })
}

/// Describes the results of a single benchmark run
pub struct RunResult {
    /// name of a test
    pub name: String,

    /// statistical summary of baseline function measurements
    pub baseline: Summary<u64>,

    /// statistical summary of candidate function measurements
    pub candidate: Summary<u64>,

    /// individual measurements of a benchmark (candidate - baseline)
    pub diff: Summary<i64>,

    /// Is difference is statistically significant
    pub significant: bool,

    /// Numbers of detected and filtered outliers
    pub outliers: usize,
}

/// Statistical summary for a given iterator of numbers.
///
/// Calculates all the information using single pass over the data. Mean and variance are calculated using
/// streaming algorithm described in [1].
///
/// [1]: Art of Computer Programming, Vol 2, page 232
#[derive(Clone, Copy)]
pub struct Summary<T> {
    pub n: usize,
    pub min: T,
    pub max: T,
    pub mean: f64,
    pub variance: f64,
}

impl<T: PartialOrd> Summary<T> {
    pub fn from<'a, C>(values: C) -> Option<Self>
    where
        C: IntoIterator<Item = &'a T>,
        T: ToPrimitive + Copy + Default + 'a,
    {
        Self::running(values.into_iter().copied()).last()
    }

    pub fn running<I>(iter: I) -> impl Iterator<Item = Summary<T>>
    where
        T: ToPrimitive + Copy + Default,
        I: Iterator<Item = T>,
    {
        RunningSummary {
            iter,
            n: 0,
            min: T::default(),
            max: T::default(),
            mean: 0.,
            s: 0.,
        }
    }
}

struct RunningSummary<T, I> {
    iter: I,
    n: usize,
    min: T,
    max: T,
    mean: f64,
    s: f64,
}

impl<T, I> Iterator for RunningSummary<T, I>
where
    T: Copy + PartialOrd,
    I: Iterator<Item = T>,
    T: ToPrimitive,
{
    type Item = Summary<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.iter.next()?;
        let fvalue = value.to_f64().expect("f64 overflow detected");

        if self.n == 0 {
            self.min = value;
            self.max = value;
        }

        if let Some(Ordering::Less) = value.partial_cmp(&self.min) {
            self.min = value;
        }
        if let Some(Ordering::Greater) = value.partial_cmp(&self.max) {
            self.max = value;
        }

        self.n += 1;
        let mean_p = self.mean;
        self.mean += (fvalue - self.mean) / self.n as f64;
        self.s += (fvalue - mean_p) * (fvalue - self.mean);
        let variance = if self.n > 1 {
            self.s / (self.n - 1) as f64
        } else {
            0.
        };

        Some(Summary {
            n: self.n,
            min: self.min,
            max: self.max,
            mean: self.mean,
            variance,
        })
    }
}

/// Outlier detection algorithm based on interquartile range
///
/// Outliers are observations are 5 IQR away from the corresponding quartile.
fn iqr_variance_thresholds(mut input: Vec<i64>) -> Option<RangeInclusive<i64>> {
    const FACTOR: i64 = 5;

    input.sort();
    let (q1, q3) = (input.len() / 4, input.len() * 3 / 4);
    if q1 >= q3 || q3 >= input.len() || input[q1] >= input[q3] {
        return None;
    }
    let iqr = input[q3] - input[q1];

    let low_threshold = input[q1] - iqr * FACTOR;
    let high_threshold = input[q3] + iqr * FACTOR;

    // Calculating the indicies of the thresholds in an dataset
    let low_threshold_idx = match input[0..q1].binary_search(&low_threshold) {
        Ok(idx) => idx,
        Err(idx) => idx,
    };

    let high_threshold_idx = match input[q3..].binary_search(&high_threshold) {
        Ok(idx) => idx,
        Err(idx) => idx,
    };

    if low_threshold_idx == 0 || high_threshold_idx >= input.len() {
        return None;
    }

    // Calculating the equal number of observations which should be removed from each "side" of observations
    let outliers_cnt = low_threshold_idx.min(input.len() - high_threshold_idx);

    Some(input[outliers_cnt]..=(input[input.len() - outliers_cnt]))
}

mod timer {
    use std::time::Instant;

    #[cfg(all(feature = "hw_timer", target_arch = "x86_64"))]
    pub(super) type ActiveTimer = x86::RdtscpTimer;

    #[cfg(not(feature = "hw_timer"))]
    pub(super) type ActiveTimer = PlatformTimer;

    pub(super) trait Timer<T> {
        fn start() -> T;
        fn stop(start_time: T) -> u64;
    }

    pub(super) struct PlatformTimer;

    impl Timer<Instant> for PlatformTimer {
        #[inline]
        fn start() -> Instant {
            Instant::now()
        }

        #[inline]
        fn stop(start_time: Instant) -> u64 {
            start_time.elapsed().as_nanos() as u64
        }
    }

    #[cfg(all(feature = "hw_timer", target_arch = "x86_64"))]
    pub(super) mod x86 {
        use super::Timer;
        use std::arch::x86_64::{__rdtscp, _mm_mfence};

        pub struct RdtscpTimer;

        impl Timer<u64> for RdtscpTimer {
            #[inline]
            fn start() -> u64 {
                unsafe {
                    _mm_mfence();
                    __rdtscp(&mut 0)
                }
            }

            #[inline]
            fn stop(start: u64) -> u64 {
                unsafe {
                    let end = __rdtscp(&mut 0);
                    _mm_mfence();
                    end - start
                }
            }
        }
    }
}

fn median_execution_time(target: &mut dyn MeasureTarget, iterations: u32) -> u64 {
    assert!(iterations >= 1);
    let measures: Vec<_> = (0..iterations).map(|_| target.measure(1)).collect();
    median(measures)
}

fn median<T: Copy + Ord + Add<Output = T> + Div<Output = T>>(mut measures: Vec<T>) -> T {
    assert!(!measures.is_empty(), "Vec is empty");
    measures.sort();
    measures[measures.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::SmallRng, RngCore, SeedableRng};
    use std::{iter::Sum, thread};

    #[test]
    fn check_summary_statistics() {
        for i in 2u32..100 {
            let range = 1..=i;
            let values = range.collect::<Vec<_>>();
            let stat = Summary::from(&values).unwrap();

            let sum = (i * (i + 1)) as f64 / 2.;
            let expected_mean = sum as f64 / i as f64;
            let expected_variance = naive_variance(values.as_slice());

            assert_eq!(stat.min, 1);
            assert_eq!(stat.n, i as usize);
            assert_eq!(stat.max, i);
            assert!(
                (stat.mean - expected_mean).abs() < 1e-5,
                "Expected close to: {}, given: {}",
                expected_mean,
                stat.mean
            );
            assert!(
                (stat.variance - expected_variance).abs() < 1e-5,
                "Expected close to: {}, given: {}",
                expected_variance,
                stat.variance
            );
        }
    }

    #[test]
    fn check_summary_statistics_types() {
        let _ = Summary::from(<&[i64]>::default());
        let _ = Summary::from(<&[u32]>::default());
        let _ = Summary::from(&Vec::<i64>::default());
    }

    #[test]
    fn check_naive_variance() {
        assert_eq!(naive_variance(&[1, 2, 3]), 1.0);
        assert_eq!(naive_variance(&[1, 2, 3, 4, 5]), 2.5);
    }

    #[test]
    fn check_running_variance() {
        let input = [1i64, 2, 3, 4, 5, 6, 7];
        let variances = Summary::running(input.into_iter())
            .map(|s| s.variance)
            .collect::<Vec<_>>();
        let expected = &[0., 0.5, 1., 1.6666, 2.5, 3.5, 4.6666];

        assert_eq!(variances.len(), expected.len());

        for (value, expected_value) in variances.iter().zip(expected) {
            assert!(
                (value - expected_value).abs() < 1e-3,
                "Expected close to: {}, given: {}",
                expected_value,
                value
            );
        }
    }

    #[test]
    fn check_running_variance_stress_test() {
        let rng = RngIterator(SmallRng::seed_from_u64(0)).map(|i| i as i64);
        let mut variances = Summary::running(rng).map(|s| s.variance);

        assert!(variances.nth(1_000_000).unwrap() > 0.)
    }

    /// Basic check of measurement code
    ///
    /// This test is quite brittle. There is no guarantee the OS scheduler will wake up the thread
    /// soon enough to meet measurement target. We try to mitigate this possibility using several strategies:
    /// 1. repeating test several times and taking median as target measurement.
    /// 2. using more liberal checking condition (allowing 1 order of magnitude error in measurement)
    #[test]
    fn check_measure_time() {
        let expected_delay = 1;
        let mut target = benchmark_fn("foo", move || {
            thread::sleep(Duration::from_millis(expected_delay))
        });

        let median = median_execution_time(target.as_mut(), 10) / NS_TO_MS as u64;
        assert!(median < expected_delay * 10);
    }

    struct RngIterator<T>(T);

    impl<T: RngCore> Iterator for RngIterator<T> {
        type Item = u32;

        fn next(&mut self) -> Option<Self::Item> {
            Some(self.0.next_u32())
        }
    }

    fn naive_variance<T>(values: &[T]) -> f64
    where
        T: Sum + Copy,
        f64: From<T>,
    {
        let n = values.len() as f64;
        let mean = f64::from(values.iter().copied().sum::<T>()) / n;
        let mut sum_of_squares = 0.;
        for value in values.into_iter().copied() {
            sum_of_squares += (f64::from(value) - mean).powi(2);
        }
        sum_of_squares / (n - 1.)
    }
}
