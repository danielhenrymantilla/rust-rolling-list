#![feature(
    box_into_raw_non_null,
    const_generics,
    type_alias_impl_trait,
)]

use ::std::{
    cell::{
        Cell,
        UnsafeCell,
    },
    iter::{self,
        FromIterator,
    },
    mem::{self,
        MaybeUninit,
    },
    ptr,
    slice,
};

type NullablePtr<T> = Option<ptr::NonNull<T>>;

struct Chunk<T, const CHUNK_CAPACITY: usize> {
    next: Cell<NullablePtr< Chunk<T, CHUNK_CAPACITY> >>,
    len: Cell<usize>,
    buffer: [UnsafeCell<MaybeUninit<T>>; CHUNK_CAPACITY]
}
impl<T, const CHUNK_CAPACITY: usize> Chunk<T, CHUNK_CAPACITY> {
    fn new (first_elem: T) -> Box<Self>
    {
        assert_ne!(CHUNK_CAPACITY, 0, "chunks CHUNK_capacity cannot be NULL!");
        let mut ret = Box::new(Self {
            next: Cell::new(None),
            len: Cell::new(1),
            buffer: unsafe {
                // # Safety
                // 
                //   - it is sound to have an uninitialized array of `MaybeUninit`s.
                MaybeUninit::uninit().assume_init()
            },
        });
        ret.buffer[0] = UnsafeCell::new(MaybeUninit::new(first_elem));
        ret
    }
}

pub
struct List<T, const CHUNK_CAPACITY: usize> {
    head: NullablePtr< Chunk<T, CHUNK_CAPACITY> >,
    last: NullablePtr< Chunk<T, CHUNK_CAPACITY> >,
}

impl<T, const CHUNK_CAPACITY: usize> List<T, CHUNK_CAPACITY> {
    #[inline]
    pub
    fn new () -> Self
    {
        assert_ne!(CHUNK_CAPACITY, 0, "CHUNK_CAPACITY cannot be nul!");
        Self {
            head: None,
            last: None,
        }
    }

    #[inline]
    fn last (self: &'_ mut Self) -> Option<&'_ Chunk<T, CHUNK_CAPACITY>>
    {
        Some(unsafe {
            // # Safety
            // 
            //   - the ptr is valid since it is the safety invariant of Self
            self.last.as_ref()?.as_ref()
        })
    }
    
    pub
    fn push (self: &'_ mut Self, elem: T)
    {
        if let Some(last) = self.last() {
            let len = last.len.get();
            if len < CHUNK_CAPACITY {
                // there is room for one
                unsafe {
                    // # Safety
                    //
                    //   - Mutation is sound because the only thing aliasing `last` is
                    //     the pointer accessible from `head`, which remains unused in all
                    //     this block.
                    //
                    //   - The `&mut Self` provides the peace of mind that this local reasoning
                    //     suffices (while also providing the `Sync`-ness of `Self`)
                    *last.buffer[len].get() = MaybeUninit::new(elem);
                }
                last.len.set(len + 1);
            } else {
                let ptr = Some(Box::into_raw_non_null(
                    Chunk::new(elem)
                ));
                last.next.set(ptr);
                self.last = ptr;
            }
        } else {
            let ptr = Some(Box::into_raw_non_null(
                Chunk::new(elem)
            ));
            self.last = ptr;
            self.head = ptr;
        }
    }

    #[inline]
    pub
    fn append (self: &'_ mut Self, other: Self)
    {
        if let Some(last) = self.last() {
            let other = mem::ManuallyDrop::new(other);
            let prev_last_next = last.next.replace(other.head);
            self.last = other.last;
            debug_assert!(prev_last_next.is_none());
        } else {
            let prev_self = mem::replace(self, other);
            // prev_self is empty, so we skip its destructor as an optimization
            debug_assert!(prev_self.head.is_none());
            mem::forget(prev_self);
        }
    }
    
    #[inline]
    pub
    fn iter<'a> (self: &'a Self) -> impl Iterator<Item = &'a T> + 'a
    {
        iter::from_fn({
            let mut state = unsafe {
                // # Safety
                //
                //   - The pointer is valid as part of the safety invariant of `Self`
                self.head
                    .as_ref()
                    .map(|head| (
                        // chunk
                        head.as_ref(),
                        // idx
                        0,
                    ))
            };
            move || Some({
                let (chunk, idx) = state?;
                let ret = unsafe {
                    // # Safety
                    //
                    //   - `idx < CHUNK_CAPACITY` as part of the safety invariant of `Self`.
                    // 
                    //   - Given that the only mutation of the `UnsafeCell` is possible through
                    //     an `&mut Self`, here the borrow ensures the value is indeed immutable
                    //     so the `&* ...get()` is sound.
                    //
                    //   - The value at `idx` is initialized as part of the safety invariant of
                    //     `Self`, so the final `&* ...as_ptr()` is also sound.
                    debug_assert!(idx < CHUNK_CAPACITY);
                    &*(&*chunk.buffer.get_unchecked(idx).get()).as_ptr()
                };
                let idx = idx + 1;
                if idx >= chunk.len.get() {
                    state = chunk.next.get().map(|ptr| (
                        unsafe {
                            // Safety: valid pointer as part of safety invariant
                            &*ptr.as_ptr()
                        },
                        0,
                    ));
                } else {
                    state = Some((chunk, idx));
                }
                ret
            })
        })
    }
}

impl<T, const CHUNK_CAPACITY: usize> Drop for List<T, CHUNK_CAPACITY> {
    #[inline]
    fn drop (self: &'_ mut Self)
    {
        if cfg!(debug_assertions) {
            self.last = None; // No more aliasing.
        }
        let mut cursor = self.head;
        while let Some(mut chunk) = cursor {
            let chunk: &mut Chunk<_, CHUNK_CAPACITY> = unsafe { chunk.as_mut() };
            cursor = chunk.next.get();
            unsafe {
                // # Safety
                //
                //   - The safety invariant of `Self` relies on `buffer[.. len]`
                //     being a slice of valid `T`s.
                //
                //   - `&mut Self` and `last` no longer used ensures that aliasing
                //     is no longer an issue either.
                let ptr: *mut T = chunk.buffer.as_mut_ptr().cast();
                ptr::drop_in_place::<[T]>(
                    slice::from_raw_parts_mut(ptr, chunk.len.get())
                );
            }
            unsafe {
                // # Safety
                //
                //   - The safety invariant of `Self` relies on the chunks having been
                //     `Box`-allocated.
                drop(Box::from_raw(chunk));
            }
        }
    }
}

impl<T, const CHUNK_CAPACITY: usize> Extend<T> for List<T, CHUNK_CAPACITY> {
    #[inline]
    fn extend<Iterable> (self: &'_ mut Self, iterable: Iterable)
    where
        Iterable : IntoIterator<Item = T>,
    {
        iterable
            .into_iter()
            .for_each(|elem| self.push(elem))
        ;
    }
}

impl<T, const CHUNK_CAPACITY: usize> FromIterator<T> for List<T, CHUNK_CAPACITY> {
    #[inline]
    fn from_iter<Iterable> (iterable: Iterable) -> Self
    where
        Iterable : IntoIterator<Item = T>,
    {
        let mut ret = Self::new();
        ret.extend(iterable);
        ret
    }
}

impl<'a, T : 'a, const CHUNK_CAPACITY: usize> IntoIterator for &'a List<T, CHUNK_CAPACITY> {
    type Item = &'a T;
    type IntoIter = impl Iterator<Item = Self::Item> + 'a;

    #[inline]
    fn into_iter (self: & 'a List<T, CHUNK_CAPACITY>) -> Self::IntoIter
    {
        self.iter()
    }
}

/* == MARKER TRAITS & Safety ==
 * Since it is not possible to mutate a `List` through a _shared_ reference to it
 * (its interior mutability being there just for soundness _w.r.t._ aliasing due
 * to the `last` field), List automagically ought to be:
 *   - RefUnwindSafe,
 *   - Sync,
 *
 * Moreover, there is no reason not to be `Send` either (why is `UnsafeCell` not Send?)
 */

// We can delegate `RefUnWindSafe`-safety to its elements
impl<T, const CHUNK_CAPACITY: usize> ::std::panic::RefUnwindSafe
    for List<T, CHUNK_CAPACITY>
where
    T : ::std::panic::RefUnwindSafe,
{}

// # Safety: As stated above, we can delegate `Sync`-safety to its elements given
// the lack of public interior mutability.
unsafe impl<T, const CHUNK_CAPACITY: usize> Sync
    for List<T, CHUNK_CAPACITY>
where
    T : Sync,
{}
// # Safety: As stated above, we can delegate `Send`-safety to its elements.
unsafe impl<T, const CHUNK_CAPACITY: usize> Send
    for List<T, CHUNK_CAPACITY>
where
    T : Send,
{}

// Can we delegate `UnwindSafe`-safety for its elements?
// Since the only moment where custom panicking code runs in the middle of potentially
// broken invariants is when `Drop` is run, it can CURRENTLY so be.
impl<T, const CHUNK_CAPACITY: usize> ::std::panic::UnwindSafe
    for List<T, CHUNK_CAPACITY>
where
    T : ::std::panic::UnwindSafe,
{}

#[cfg(test)]
mod tests;
