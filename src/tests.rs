use super::*;
use self::leak_checker::{LeakChecker, ToOwned};
#[test]
fn basic ()
{
    const ELEMS: [i32; 5] = [1, 2, 3, 4, 5];
    fn check_with_capacity<const CAPACITY: usize> (arena: &'_ LeakChecker)
    {
        dbg!(CAPACITY);
        let list: List<_, CAPACITY> =
            ELEMS
                .iter()
                .map(|&x| arena.alloc(x))
                .collect()
        ;
        assert_eq!(
            list.iter().map(ToOwned::to_owned).collect::<Vec<_>>(),
            ELEMS,
        );
    }
    LeakChecker::with(|arena| {
        check_with_capacity::<1>(arena);
        check_with_capacity::<2>(arena);
        check_with_capacity::<3>(arena);
        check_with_capacity::<4>(arena);
        check_with_capacity::<5>(arena);
        check_with_capacity::<6>(arena);
    })
}

#[test]
fn append ()
{
    let mut elems1 = Vec::from_iter(0 .. 103);
    let elems2 = Vec::from_iter(598 .. 700);
    LeakChecker::with(|arena| {
        let alloc =
            |&x| arena.alloc(x)
        ;
        let mut list: List<_, 37> = elems1.iter().map(alloc).collect();
        list.append(elems2.iter().map(alloc).collect());
        elems1.extend(elems2);
        assert_eq!(
            list.iter().map(ToOwned::to_owned).collect::<Vec<_>>(),
            elems1,
        );
    })
}

mod leak_checker {
    use ::core::cell::Cell;

    const CAPACITY: usize = 8096;

    pub(in super)
    struct LeakChecker {
        allocated_slots: [Cell<bool>; CAPACITY],
        next_free: Cell<usize>,
    }

    pub
    trait ToOwned {
        type Owned;

        fn to_owned (self: &'_ Self) -> Self::Owned
        ;
    }

    impl LeakChecker {
        pub
        fn with (f: impl FnOnce(&'_ Self))
        {
            let this = Self::new();
            f(&this);
            this.assert_no_leaks();
        }

        pub
        fn alloc<'a, T : Clone + 'a> (self: &'a Self, value: T)
          -> impl ToOwned<Owned = T> + 'a
        {
            let next_free = self.next_free.get();
            self.next_free.set(next_free + 1);
            let slot =
                self.allocated_slots
                    .get(next_free)
                    .expect("`LeakChecker` ran out of capacity")
            ;
            slot.set(true); // allocate
            return Ret { value, slot };

            // where
            pub
            struct Ret<'a, T : Clone> {
                slot: &'a Cell<bool>,
                value: T,
            }
            impl<T : Clone> ToOwned for Ret<'_, T> {
                type Owned = T;
                fn to_owned (self: &'_ Self) -> T
                {
                    self.value.clone()
                }
            }
            impl<T : Clone> Drop for Ret<'_, T> {
                fn drop (self: &'_ mut Self)
                {
                    if ::std::thread::panicking() { return; }
                    assert_eq!(
                        self.slot.replace(false),
                        true, // freeing an allocated slot
                        "Double free!",
                    );
                }
            }
        }

        pub(in self) // private
        fn new () -> Self
        {
            Self {
                allocated_slots: unsafe { ::core::mem::zeroed() },
                next_free: Cell::new(0),
            }
        }

        pub(in self)
        fn assert_no_leaks (self: &'_ Self)
        {
            if ::std::thread::panicking() { return; }
            assert_eq!(
                self.allocated_slots
                    .iter()
                    .map(|allocated| u64::from(allocated.get()))
                    .sum::<u64>()
                ,
                0,
                "Memory was leaked",
            );
        }
    }
}