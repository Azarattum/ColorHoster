use std::{
    ops::{
        Bound, Index, IndexMut, Range, RangeBounds, RangeFrom, RangeFull, RangeInclusive, RangeTo,
        RangeToInclusive,
    },
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

#[derive(Debug, Clone)]
pub struct Report<const N: usize> {
    data: [u8; N],
}

impl<const N: usize> Report<N> {
    pub fn new() -> Self {
        Report { data: [0; N] }
    }

    pub fn into_inner(self) -> [u8; N] {
        self.data
    }

    pub fn len(&self) -> usize {
        return N - 1;
    }

    fn adjust_range<R: RangeBounds<usize>>(&self, range: R) -> (usize, usize) {
        let start = match range.start_bound() {
            Bound::Included(&s) => s,
            Bound::Excluded(&s) => s + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&e) => e + 1,
            Bound::Excluded(&e) => e,
            Bound::Unbounded => self.len(),
        };

        (start + 1, end + 1)
    }
}

impl<const N: usize> Index<usize> for Report<N> {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.data[index + 1]
    }
}

impl<const N: usize> IndexMut<usize> for Report<N> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.data[index + 1]
    }
}

impl<const N: usize, R: ReportRange> std::ops::Index<R> for Report<N> {
    type Output = [u8];

    fn index(&self, range: R) -> &Self::Output {
        let (start, end) = self.adjust_range(range);
        &self.data[start..end]
    }
}

impl<const N: usize, R: ReportRange> std::ops::IndexMut<R> for Report<N> {
    fn index_mut(&mut self, range: R) -> &mut Self::Output {
        let (start, end) = self.adjust_range(range);
        &mut self.data[start..end]
    }
}

trait ReportRange: RangeBounds<usize> {}
impl ReportRange for Range<usize> {}
impl ReportRange for RangeTo<usize> {}
impl ReportRange for RangeFrom<usize> {}
impl ReportRange for RangeInclusive<usize> {}
impl ReportRange for RangeToInclusive<usize> {}
impl ReportRange for RangeFull {}

pub struct ReportFutureInner<const N: usize> {
    pub data: Option<[u8; N]>,
    pub waker: Option<Waker>,
}

pub type FutureReportState<const N: usize> = Arc<Mutex<ReportFutureInner<N>>>;

pub struct FutureReport<const N: usize> {
    state: FutureReportState<N>,
}

impl<const N: usize> FutureReport<N> {
    pub fn new_state() -> FutureReportState<N> {
        Arc::new(Mutex::new(ReportFutureInner {
            data: None,
            waker: None,
        }))
    }

    pub fn from_state(state: FutureReportState<N>) -> Self {
        Self { state }
    }
}

impl<const N: usize> Future for FutureReport<N> {
    type Output = [u8; N];

    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();

        if let Some(data) = state.data.take() {
            Poll::Ready(data)
        } else {
            state.waker = Some(ctx.waker().clone());
            Poll::Pending
        }
    }
}
