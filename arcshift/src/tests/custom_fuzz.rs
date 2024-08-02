//! Fuzz-test cases with focus on sending instances between threads

use std::collections::HashSet;
use super::{atomic, InstanceSpy2, model};
use crate::tests::leak_detection::{SpyOwner2};
use crate::{ArcShift, ArcShiftLight, get_refcount, MAX_ROOTS};
use std::fmt::Debug;
use std::hash::Hash;
use crossbeam_channel::bounded;
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};

enum PipeItem<T: 'static> {
    Shift(ArcShift<T>),
    Root(ArcShiftLight<T>),
}

fn run_multi_fuzz<T: Clone + Hash + Eq + 'static + Debug + Send + Sync>(
    rng: &mut StdRng,
    mut constructor: impl FnMut() -> T,
) {
    let cmds = make_commands::<T>(rng, &mut constructor);
    let mut all_possible: HashSet<T> = HashSet::new();
    for cmd in cmds.iter() {
        if let FuzzerCommand::CreateUpdateArc(_, val) | FuzzerCommand::CreateArcLight(_, val) | FuzzerCommand::CreateUpdateArcLight(_, val) =
            cmd
        {
            all_possible.insert(val.clone());
        }
    }
    let mut batches = Vec::new();
    let mut senders = vec![];
    let mut receivers = vec![];
    for _ in 0..3 {
        let (sender, receiver) = bounded::<PipeItem<T>>(cmds.len());
        senders.push(sender);
        receivers.push(receiver);
        batches.push(Vec::new());
    }
    for cmd in cmds {
        batches[cmd.batch() as usize].push(cmd);
    }

    let mut jhs = Vec::new();

    let initial = constructor();
    all_possible.insert(initial.clone());

    let senders = std::sync::Arc::new(senders);
    let all_possible: std::sync::Arc<HashSet<T>> = std::sync::Arc::new(all_possible);
    //println!("Start iteration");
    let start_arc_light = ArcShiftLight::new(initial);
    for (threadnr, (cmds, receiver)) in batches.into_iter().zip(receivers).enumerate() {
        let thread_senders = std::sync::Arc::clone(&senders);
        let thread_all_possible: std::sync::Arc<HashSet<T>> =
            std::sync::Arc::clone(&all_possible);
        let start_arc0 = start_arc_light.upgrade();
        let start_arc_light = start_arc_light.clone();
        let jh = atomic::thread::Builder::new().name(format!("thread{}", threadnr)).spawn(move || {
            let mut curval: Option<ArcShift<T>> = Some(start_arc0);
            let mut curvalroot: Option<ArcShiftLight<T>> = Some(start_arc_light);
            for cmd in cmds {
                if let Ok(val) = receiver.try_recv() {
                    match val {
                        PipeItem::Shift(shift) => {
                            curval = Some(shift);
                        }
                        PipeItem::Root(root) => {
                            curvalroot = Some(root);
                        }
                    }
                }
                //println!("Apply cmd {:?}: {:?}", cmd, atomic::thread::current().id());
                match cmd {
                    FuzzerCommand::CreateUpdateArc(_, val) => {
                        if let Some(curval) = &mut curval {
                            curval.update_shared(val);
                        } else {
                            curval = Some(ArcShift::new(val));
                        }
                    }
                    FuzzerCommand::ReadArc { .. } => {
                        if let Some(curval) = curval.as_mut() {
                            let actual = curval.get();
                            if !thread_all_possible.contains(actual) {
                                panic!("Unexpected value in thread {}, got {:?} which is not in {:?}", threadnr, actual, thread_all_possible);
                            }
                        }
                    }
                    FuzzerCommand::SharedReadArc { .. } => {
                        if let Some(curval) = curval.as_mut() {
                            let actual = curval.shared_get();
                            if !thread_all_possible.contains(actual) {
                                panic!("Unexpected value in thread {}, got {:?} which is not in {:?}", threadnr, actual, thread_all_possible);
                            }
                        }
                    }
                    FuzzerCommand::CloneArc { from: _, to: _ } => {
                        if let Some(curval) = curval.as_mut() {
                            let cloned = curval.clone();
                            thread_senders[threadnr].send(PipeItem::Shift(cloned)).unwrap();
                        }
                    }
                    FuzzerCommand::DropArc(_) => {
                        curval = None;
                    }
                    FuzzerCommand::CreateArcLight(_, val) => {
                        if let Some(curvalroot) = curvalroot.as_ref() {
                            curvalroot.update_shared(val);
                        } else {
                            curvalroot = Some(ArcShiftLight::new(val));
                        }
                    }
                    FuzzerCommand::CloneArcLight { .. } => {
                        if let Some(curvalroot) = curvalroot.as_mut() {
                            let cloned = curvalroot.clone();
                            thread_senders[threadnr].send(PipeItem::Root(cloned)).unwrap();
                        }
                    }
                    FuzzerCommand::CreateUpdateArcLight(_, val) => {
                        if let Some(curvalroot) = &mut curvalroot {
                            curvalroot.update_shared(val);
                        } else {
                            curvalroot = Some(ArcShiftLight::new(val));
                        }
                    }
                    FuzzerCommand::UpgradeLight(_) => {
                        if let Some(root) = curvalroot.as_ref() {
                            curval = Some(root.upgrade());
                        }
                    }
                    FuzzerCommand::DowngradeLight(_) => {
                        if let Some(arc) = curval.as_ref() {
                            curvalroot = Some(arc.make_light());
                        }
                    }
                    FuzzerCommand::DropLight(_) => {
                        curvalroot = None;
                    }
                }
            }
        }).unwrap();
        jhs.push(jh);
    }
    for jh in jhs {
        jh.join().unwrap();
    }
}

#[derive(Debug)]
enum FuzzerCommand<T> {
    CreateUpdateArc(u8, T),
    CreateArcLight(u8, T),
    ReadArc { arc: u8 },
    SharedReadArc { arc: u8 },
    CloneArc { from: u8, to: u8 },
    CloneArcLight { from: u8, to: u8 },
    CreateUpdateArcLight(u8, T),
    DropArc(u8),
    UpgradeLight(u8),
    DowngradeLight(u8),
    DropLight(u8),
}
impl<T> FuzzerCommand<T> {
    fn batch(&self) -> u8 {
        match self {
            FuzzerCommand::CreateUpdateArc(chn, _) => *chn,
            FuzzerCommand::ReadArc { arc } => *arc,
            FuzzerCommand::SharedReadArc { arc } => *arc,
            FuzzerCommand::CloneArc { from, .. } => *from,
            FuzzerCommand::DropArc(chn) => *chn,
            FuzzerCommand::CreateArcLight(chn, _) => *chn,
            FuzzerCommand::CloneArcLight { from, .. } => *from,
            FuzzerCommand::CreateUpdateArcLight(chn, _) => *chn,
            FuzzerCommand::UpgradeLight(chn) => *chn,
            FuzzerCommand::DowngradeLight(chn) => *chn,
            FuzzerCommand::DropLight(chn) => *chn,
        }
    }
}

#[test]
#[cfg(
    not(any(loom, miri, feature = "shuttle"))
)] //Neither loom nor shuttle allows this many iterations
#[should_panic(expected = "Max limit of ArcShiftLight clones (524288) was reached")]
fn check_too_many_roots() {
    model(|| {
        let mut temp = vec![];
        let light = ArcShiftLight::new(1u8);
        for _ in 0..MAX_ROOTS {
            temp.push(light.clone());
            atomic::spin_loop();
        }
    });
}
#[test]
#[cfg(
    not(miri)
)] // We shouldn't run miri on this, since this test uses unsafe code to leak memory.
#[should_panic(expected = "Max limit of ArcShiftLight clones (524288) was reached")]
fn check_too_many_roots2() {
    model(|| {
        let mut temp = vec![];
        let light = ArcShiftLight::new(1u8);
        // When running under 'shuttle', we can't do too many steps, so we can't
        // exhaust all MAX_ROOTS-items naturally, we have to cheat like this.
        get_refcount(light.item).fetch_add(MAX_ROOTS - 2, atomic::Ordering::SeqCst);
        for _ in 0..10 {
            temp.push(light.clone());
            atomic::spin_loop();
        }
    });
}

fn run_fuzz<T: Clone + Hash + Eq + 'static + Debug + Send + Sync>(
    rng: &mut StdRng,
    mut constructor: impl FnMut() -> T,
) {
    let cmds = make_commands::<T>(rng, &mut constructor);
    let mut arcs: [Option<ArcShift<T>>; 3] = [(); 3].map(|_| None);
    let mut arcroots: [Option<ArcShiftLight<T>>; 3] = [(); 3].map(|_| None);
    debug_println!("Starting fuzzrun");
    for cmd in cmds {
        debug_println!("\n=== Applying cmd: {:?} ===", cmd);
        match cmd {
            FuzzerCommand::CreateUpdateArc(chn, val) => {
                if let Some(arc) = &mut arcs[chn as usize] {
                    arc.update_shared(val);
                } else {
                    arcs[chn as usize] = Some(ArcShift::new(val));
                }
            }
            FuzzerCommand::ReadArc { arc } => {
                if let Some(actual) = &mut arcs[arc as usize] {
                    let actual_val = actual.get();

                    std::hint::black_box(actual_val);
                }
            }
            FuzzerCommand::SharedReadArc { arc } => {
                if let Some(actual) = &mut arcs[arc as usize] {
                    let actual_val = actual.shared_get();

                    std::hint::black_box(actual_val);
                }
            }
            FuzzerCommand::CloneArc { from, to } => {
                let clone = arcs[from as usize].clone();
                arcs[to as usize] = clone;
            }
            FuzzerCommand::DropArc(chn) => {
                arcs[chn as usize] = None;
            }
            FuzzerCommand::CreateArcLight(chn, val) => {
                arcroots[chn as usize] = Some(ArcShiftLight::new(val));
            }
            FuzzerCommand::CloneArcLight { from, to } => {
                let clone = arcroots[from as usize].clone();
                arcroots[to as usize] = clone;
            }
            FuzzerCommand::CreateUpdateArcLight(chn, val) => {
                if let Some(arc) = &mut arcroots[chn as usize] {
                    arc.update_shared(val);
                } else {
                    arcroots[chn as usize] = Some(ArcShiftLight::new(val));
                }
            }
            FuzzerCommand::UpgradeLight(chn) => {
                if let Some(root) = arcroots[chn as usize].as_ref() {
                    arcs[chn as usize] = Some(root.upgrade());
                }
            }
            FuzzerCommand::DowngradeLight(chn) => {
                if let Some(arc) = arcs[chn as usize].as_ref() {
                    arcroots[chn as usize] = Some(arc.make_light());
                }
            }
            FuzzerCommand::DropLight(chn) => {
                arcs[chn as usize] = None;
            }
        }
    }
    debug_println!("=== No more commands ===");
}

fn make_commands<T: Clone + Eq + Hash + Debug>(
    rng: &mut StdRng,
    constructor: &mut impl FnMut() -> T,
) -> Vec<FuzzerCommand<T>> {
    let mut ret = Vec::new();
    for _x in 0..50 {
        match rng.gen_range(0..12) {
            0 => {
                let chn = rng.gen_range(0..3);
                let val = constructor();
                ret.push(FuzzerCommand::CreateUpdateArc(chn, val.clone()));
            }
            1 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::ReadArc { arc: chn });
            }
            2 => {
                //Clone
                let from = rng.gen_range(0..3);
                let mut to = rng.gen_range(0..3);
                if from == to {
                    to = (from + 1) % 3;
                }

                ret.push(FuzzerCommand::CloneArc { from, to });
            }
            3 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::DropArc(chn));
            }
            4 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::CreateArcLight(chn, constructor()));
            }
            5 => {
                let chn = rng.gen_range(0..3);
                let val = constructor();
                ret.push(FuzzerCommand::CreateArcLight(chn, val.clone()));
            }
            6 => {
                let from = rng.gen_range(0..3);
                let mut to = rng.gen_range(0..3);
                if from == to {
                    to = (from + 1) % 3;
                }

                ret.push(FuzzerCommand::CloneArcLight { from, to });
            }
            7 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::UpgradeLight(chn));
            }
            8 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::DowngradeLight(chn));
            }
            9 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::SharedReadArc { arc: chn });
            }
            10 => {
                let chn = rng.gen_range(0..3);
                let val = constructor();
                ret.push(FuzzerCommand::CreateUpdateArcLight(chn, val.clone()));
            }
            11 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::DropLight(chn));
            }
            _ => unreachable!(),
        }
    }
    ret
}

#[test]
fn generic_thread_fuzzing_all() {
    #[cfg(miri)]
    const COUNT: u64 = 10;
    #[cfg(any(loom, feature = "shuttle"))]
    const COUNT: u64 = 1000;
    let statics = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    #[cfg(not(any(loom, miri, feature = "shuttle")))]
    const COUNT: u64 = 100000;
    for i in 0..COUNT {
        println!("--- Seed {} ---", i);
        model(move || {
            let mut rng = StdRng::seed_from_u64(i);
            let mut counter = 0usize;
            let owner = std::sync::Arc::new(SpyOwner2::new());
            let owner_ref = owner.clone();
            run_multi_fuzz(&mut rng, move || -> InstanceSpy2 {
                counter += 1;
                owner_ref.create(statics[counter % 10])
            });
            owner.validate();
        });
    }
}

#[test]
#[cfg(
    all(not(loom), not(feature = "shuttle"))
)] //No point in running loom on this test, it's not multi-threaded
fn generic_fuzzing_all() {
    #[cfg(miri)]
    const COUNT: u64 = 100;
    #[cfg(not(any(miri)))]
    const COUNT: u64 = 50000;
    for i in 0..COUNT {
        model(move || {
            let mut rng = StdRng::seed_from_u64(i);
            let mut counter = 0u32;
            debug_println!("Running seed {}", i);
            run_fuzz(&mut rng, move || -> u32 {
                counter += 1;
                counter
            });
        });
    }
}
#[test]
fn generic_fuzzing_159() {
    let seed = 159;
    model(move || {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut counter = 0u32;
        debug_println!("Running seed {}", seed);
        run_fuzz(&mut rng, move || -> u32 {
            counter += 1;
            counter
        });
    })
}
#[test]
fn generic_fuzzing_53014() {
    let seed = 53014;
    model(move || {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut counter = 0u32;
        debug_println!("Running seed {}", seed);
        run_fuzz(&mut rng, move || -> u32 {
            counter += 1;
            counter
        });
    })
}
#[test]
fn generic_fuzzing_3817879() {
    let seed = 3817879;
    model(move || {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut counter = 0u32;
        debug_println!("Running seed {}", seed);
        run_fuzz(&mut rng, move || -> u32 {
            counter += 1;
            counter
        });
    })
}
