use arcshift::{ArcShiftLight};

fn main() {

    let arcshift_light = ArcShiftLight::new(vec![]);
    let mut arcshift = arcshift_light.upgrade();

    drop(arcshift_light);
    arcshift.try_get_mut().unwrap().push("testing".to_string());

    arcshift.update(vec!["other payload".to_string()]);

    assert_eq!(arcshift.get(), &vec!["other payload".to_string()]);
}
