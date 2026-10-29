use crate::widgets::Page;

pub fn build(page: &Page) {
    crate::sections::diagnose::build(page);
}
