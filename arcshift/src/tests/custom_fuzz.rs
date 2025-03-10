//! Fuzz-test cases with focus on sending instances between threads

use super::{atomic, model, model2, InstanceSpy2};
use crate::tests::leak_detection::SpyOwner2;
use crate::{ArcShift, ArcShiftWeak};
use crossbeam_channel::bounded;
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

enum PipeItem<T: 'static> {
    Shift(ArcShift<T>),
    Root(ArcShiftWeak<T>),
}

fn run_multi_fuzz<T: Clone + Hash + Eq + 'static + Debug + Send + Sync>(
    rng: &mut StdRng,
    mut constructor: impl FnMut() -> T,
) {
    let cmds = make_commands::<T>(rng, &mut constructor);
    let mut all_possible: HashSet<T> = HashSet::new();
    for cmd in cmds.iter() {
        if let FuzzerCommand::CreateUpdateArc(_, val) = cmd {
            all_possible.insert(val.clone());
        }
    }
    debug_println!("Cmds: {:?}", cmds);
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
    let start_arc0 = ArcShift::new(initial);
    for (threadnr, (cmds, receiver)) in batches.into_iter().zip(receivers).enumerate() {
        let thread_senders = std::sync::Arc::clone(&senders);
        let thread_all_possible: std::sync::Arc<HashSet<T>> = std::sync::Arc::clone(&all_possible);
        let start_arc0 = start_arc0.clone();
        let start_arc_light = ArcShift::downgrade(&start_arc0);
        let jh = atomic::thread::Builder::new().name(format!("thread{}", threadnr+1)).spawn(move || {
            let mut curval: Option<ArcShift<T>> = Some(start_arc0);
            let mut curvalweak: Option<ArcShiftWeak<T>> = Some(start_arc_light);
            for cmd in cmds {
                if let Ok(val) = receiver.try_recv() {
                    match val {
                        PipeItem::Shift(shift) => {
                            curval = Some(shift);
                        }
                        PipeItem::Root(root) => {
                            curvalweak = Some(root);
                        }
                    }
                }
                debug_println!(" === Apply cmd {:?}: {:?} ===", cmd, atomic::thread::current().id());
                //loom::sync::atomic::spin_loop_hint();
                //atomic::spin_loop();
                match cmd {
                    FuzzerCommand::CreateUpdateArc(_, val) => {
                        if let Some(curval) = &mut curval {
                            curval.update(val);
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
                    FuzzerCommand::CloneArcLight { .. } => {
                        if let Some(curvalroot) = curvalweak.as_mut() {
                            let cloned = curvalroot.clone();
                            thread_senders[threadnr].send(PipeItem::Root(cloned)).unwrap();
                        }
                    }
                    FuzzerCommand::UpgradeLight(_) => {
                        if let Some(root) = curvalweak.as_ref() {
                            curval = root.upgrade();
                        }
                    }
                    FuzzerCommand::DowngradeLight(_) => {
                        if let Some(arc) = curval.as_ref() {
                            curvalweak = Some(ArcShift::downgrade(&arc));
                        }
                    }
                    FuzzerCommand::DropLight(_) => {
                        curvalweak = None;
                    }
                    FuzzerCommand::IntoInner(_) => {
                        if let Some(t) = curval.take() {
                            let _ = t.try_into_inner();
                        }
                    }
                    FuzzerCommand::GetMut(_) => {
                        if let Some(t) = curval.as_mut() {
                            let _ = t.try_get_mut();
                        }
                    }
                }
            }
        }).unwrap();
        jhs.push(jh);
    }
    for jh in jhs {
        jh.join().unwrap();
    }
    drop(senders);
    debug_println!("Validating final arc0: {:x?}", start_arc0.item.as_ptr());
    unsafe { ArcShift::debug_validate(&[&start_arc0], &[]) };
}

#[derive(Debug)]
enum FuzzerCommand<T> {
    CreateUpdateArc(u8, T),
    ReadArc { arc: u8 },
    SharedReadArc { arc: u8 },
    CloneArc { from: u8, to: u8 },
    CloneArcLight { from: u8, to: u8 },
    DropArc(u8),
    UpgradeLight(u8),
    DowngradeLight(u8),
    IntoInner(u8),
    GetMut(u8),
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
            FuzzerCommand::CloneArcLight { from, .. } => *from,
            FuzzerCommand::UpgradeLight(chn) => *chn,
            FuzzerCommand::DowngradeLight(chn) => *chn,
            FuzzerCommand::DropLight(chn) => *chn,
            FuzzerCommand::IntoInner(chn) => *chn,
            FuzzerCommand::GetMut(chn) => *chn,
        }
    }
}

fn run_fuzz<T: Clone + Hash + Eq + 'static + Debug + Send + Sync>(
    rng: &mut StdRng,
    mut constructor: impl FnMut() -> T,
) {
    let cmds = make_commands::<T>(rng, &mut constructor);
    let mut arcs: [Option<ArcShift<T>>; 3] = [(); 3].map(|_| None);
    let mut arcroots: [Option<ArcShiftWeak<T>>; 3] = [(); 3].map(|_| None);
    debug_println!("Starting fuzzrun");
    for cmd in cmds {
        debug_println!("\n=== Applying cmd: {:?} ===", cmd);
        match cmd {
            FuzzerCommand::CreateUpdateArc(chn, val) => {
                if let Some(arc) = &mut arcs[chn as usize] {
                    arc.update(val);
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
            FuzzerCommand::CloneArcLight { from, to } => {
                let clone = arcroots[from as usize].clone();
                arcroots[to as usize] = clone;
            }
            FuzzerCommand::UpgradeLight(chn) => {
                if let Some(root) = arcroots[chn as usize].as_ref() {
                    arcs[chn as usize] = root.upgrade();
                }
            }
            FuzzerCommand::DowngradeLight(chn) => {
                if let Some(arc) = arcs[chn as usize].as_ref() {
                    arcroots[chn as usize] = Some(ArcShift::downgrade(&arc));
                }
            }
            FuzzerCommand::DropLight(chn) => {
                arcs[chn as usize] = None;
            }
            FuzzerCommand::IntoInner(chn) => {
                if let Some(x) = arcs[chn as usize].take() {
                    let _ = x.try_into_inner();
                }
            }
            FuzzerCommand::GetMut(chn) => {
                if let Some(x) = arcs[chn as usize].as_mut() {
                    let _ = x.try_get_mut();
                }
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

    #[cfg(not(loom))]
    const COUNT: usize = 15;
    #[cfg(loom)]
    const COUNT: usize = 10;

    for _x in 0..COUNT {
        match rng.gen_range(0..9) {
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
                let from = rng.gen_range(0..3);
                let mut to = rng.gen_range(0..3);
                if from == to {
                    to = (from + 1) % 3;
                }

                ret.push(FuzzerCommand::CloneArcLight { from, to });
            }
            5 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::UpgradeLight(chn));
            }
            6 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::DowngradeLight(chn));
            }
            7 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::SharedReadArc { arc: chn });
            }
            8 => {
                let chn = rng.gen_range(0..3);
                ret.push(FuzzerCommand::DropLight(chn));
            }
            _ => unreachable!(),
        }
    }
    ret
}
#[test]
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_thread_fuzzing_57() {
    #[cfg(miri)]
    const COUNT: u64 = 30;
    #[cfg(any(loom))]
    const COUNT: u64 = 100;
    #[cfg(all(feature = "shuttle", not(coverage)))]
    const COUNT: u64 = 1000;
    #[cfg(coverage)]
    const COUNT: u64 = 10;

    let statics = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    #[cfg(not(any(loom, miri, feature = "shuttle", coverage)))]
    const COUNT: u64 = 100000;
    {
        let i = 57;
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
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_thread_fuzzing_all() {
    generic_thread_fuzzing_all_impl(None, None)
}
#[test]
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_thread_fuzzing_repro1() {
    generic_thread_fuzzing_all_impl(Some(249), None)
}
#[test]
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_thread_fuzzing_repro2() {
    generic_thread_fuzzing_all_impl(Some(72), None)
}

fn generic_thread_fuzzing_all_impl(seed: Option<u64>, repro: Option<&str>) {
    #[cfg(miri)]
    const COUNT: u64 = 30;
    #[cfg(any(loom))]
    const COUNT: u64 = 100;
    #[cfg(all(feature = "shuttle", not(coverage)))]
    const COUNT: u64 = 1000;
    #[cfg(coverage)]
    const COUNT: u64 = 10;

    let statics = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    #[cfg(not(any(loom, miri, feature = "shuttle", coverage)))]
    const COUNT: u64 = 10000;
    let range = if let Some(seed) = seed {
        seed..seed + 1
    } else {
        0..COUNT
    };
    for i in range {
        model2(
            move || {
                //println!("--- Seed {} ---", i);
                let mut rng = StdRng::seed_from_u64(i);
                let mut counter = 0usize;
                let owner = std::sync::Arc::new(SpyOwner2::new());
                let owner_ref = owner.clone();
                run_multi_fuzz(&mut rng, move || -> InstanceSpy2 {
                    counter += 1;
                    owner_ref.create(statics[counter % 10])
                });
                owner.validate();
            },
            repro,
        );
    }
}

#[test]
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_thread_fuzzing_121() {
    {
        let i = 121;
        let statics = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
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
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_thread_fuzzing_21() {
    {
        let i = 21;
        let statics = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
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
#[cfg(not(feature = "disable_slow_tests"))]
fn generic_thread_fuzzing_8() {
    {
        let i = 8;
        let statics = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
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
#[cfg(all(not(loom), not(feature = "shuttle")))] //No point in running loom on this test, it's not multi-threaded
fn generic_fuzzing_all() {
    #[cfg(miri)]
    const COUNT: u64 = 100;
    #[cfg(not(any(miri)))]
    const COUNT: u64 = 50000;
    for i in 0..COUNT {

        let mut rng = StdRng::seed_from_u64(i);
        let mut counter = 0u32;
        debug_println!("Running seed {}", i);
        run_fuzz(&mut rng, move || -> u32 {
            counter += 1;
            counter
        });

    }
}

#[cfg(not(any(loom, feature = "shuttle")))]
#[test]
fn generic_fuzzing_159() {
    let seed = 159;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut counter = 0u32;
    debug_println!("Running seed {}", seed);
    run_fuzz(&mut rng, move || -> u32 {
        counter += 1;
        counter
    });
}

#[cfg(not(any(loom, feature = "shuttle")))]
#[test]
fn generic_fuzzing_121() {
    let seed = 121;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut counter = 0u32;
    debug_println!("Running seed {}", seed);
    run_fuzz(&mut rng, move || -> u32 {
        counter += 1;
        counter
    });
}
#[cfg(not(any(loom, feature = "shuttle")))]
#[test]
fn generic_fuzzing_53014() {
    let seed = 53014;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut counter = 0u32;
    debug_println!("Running seed {}", seed);
    run_fuzz(&mut rng, move || -> u32 {
        counter += 1;
        counter
    });
}

#[cfg(not(any(loom, feature = "shuttle")))]
#[test]
fn generic_fuzzing_3817879() {
    let seed = 3817879;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut counter = 0u32;
    debug_println!("Running seed {}", seed);
    run_fuzz(&mut rng, move || -> u32 {
        counter += 1;
        counter
    });
}
