//! `xtask compare-jobs`: where a page's ink is, and how two pages differ.
use tmc3500_core::raster::{Ink, InkPage};
use xtask::{compare_pages, ink_bbox};

fn page(dots: &[(Ink, u32, u32)]) -> InkPage {
    let mut p = InkPage::new(100, 50).unwrap();
    for &(ink, x, y) in dots {
        p.set(ink, x, y, 2);
    }
    p
}

#[test]
fn bbox_covers_every_ink_and_is_none_for_a_blank_page() {
    assert_eq!(ink_bbox(&page(&[])), None);
    let p = page(&[(Ink::C, 10, 20), (Ink::K, 3, 40), (Ink::Y, 99, 5)]);
    assert_eq!(ink_bbox(&p), Some([3, 5, 99, 40]));
}

#[test]
fn identical_pages_differ_nowhere() {
    let a = page(&[(Ink::M, 7, 8), (Ink::K, 9, 9)]);
    let c = compare_pages(&a, &a);
    assert_eq!(c.differing_dots, 0);
    assert_eq!(c.offset, Some([0, 0]));
}

#[test]
fn a_shifted_page_reports_the_shift_and_the_dots_that_moved() {
    let a = page(&[(Ink::M, 7, 8), (Ink::K, 9, 9)]);
    let b = page(&[(Ink::M, 9, 5), (Ink::K, 11, 6)]);
    let c = compare_pages(&a, &b);
    assert_eq!(c.offset, Some([2, -3]));
    assert_eq!(c.differing_dots, 4);
    // a different drop size on the same dot counts too
    let mut d = a.clone();
    d.set(Ink::M, 7, 8, 1);
    assert_eq!(compare_pages(&a, &d).differing_dots, 1);
}

#[test]
fn pages_of_different_sizes_compare_over_both() {
    let a = page(&[(Ink::K, 1, 1)]);
    let mut b = InkPage::new(120, 50).unwrap();
    b.set(Ink::K, 1, 1, 2);
    b.set(Ink::K, 110, 1, 2);
    let c = compare_pages(&a, &b);
    assert_eq!(c.differing_dots, 1);
}
