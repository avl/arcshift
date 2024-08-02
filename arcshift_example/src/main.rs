use arcshift::{ArcShiftLight};

fn main() {

    let arcshift_light = ArcShiftLight::new("Hello".to_string());
    let mut arcshift = arcshift_light.upgrade();
    arcshift.update("Hello World!".to_string());

    assert_eq!(arcshift.get(), "Hello World!");
}

