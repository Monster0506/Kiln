pub mod deprecated;
pub mod derive;
pub mod indirect;
pub mod inline_;
pub mod static_;
pub mod test_;

use crate::annotations::ProcessorRegistry;

pub fn register_all(r: &mut ProcessorRegistry) {
    static_::register(r);
    derive::register(r);
    test_::register(r);
    indirect::register(r);
    deprecated::register(r);
    inline_::register(r);
}
