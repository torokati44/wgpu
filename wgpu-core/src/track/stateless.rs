/*! Stateless Trackers
 *
 * Stateless trackers don't have any state, so make no
 * distinction between a usage scope and a full tracker.
!*/

use std::{marker::PhantomData, sync::Arc};

use crate::{
    hal_api::HalApi,
    id::{TypedId, Valid},
    resource::Resource,
    storage::Storage,
    track::ResourceMetadata,
};

/// Stores all the resources that a bind group stores.
#[derive(Debug)]
pub(crate) struct StatelessBindGroupSate<Id: TypedId, T: Resource<Id>> {
    resources: Vec<(Valid<Id>, Arc<T>)>,
}

impl<Id: TypedId, T: Resource<Id>> StatelessBindGroupSate<Id, T> {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
        }
    }

    /// Optimize the buffer bind group state by sorting it by ID.
    ///
    /// When this list of states is merged into a tracker, the memory
    /// accesses will be in a constant assending order.
    pub(crate) fn optimize(&mut self) {
        self.resources
            .sort_unstable_by_key(|&(id, _)| id.0.unzip().0);
    }

    /// Returns a list of all resources tracked. May contain duplicates.
    pub fn used(&self) -> impl Iterator<Item = Valid<Id>> + '_ {
        self.resources.iter().map(|&(id, _)| id)
    }

    /// Returns a list of all resources tracked. May contain duplicates.
    pub fn used_resources(&self) -> impl Iterator<Item = Arc<T>> + '_ {
        self.resources
            .iter()
            .map(|&(_, ref resource)| resource.clone())
    }

    /// Adds the given resource.
    pub fn add_single<'a>(&mut self, storage: &'a Storage<T, Id>, id: Id) -> Option<&'a T> {
        let resource = storage.get(id).ok()?;

        self.resources.push((Valid(id), resource.clone()));

        Some(resource)
    }
}

/// Stores all resource state within a command buffer or device.
#[derive(Debug)]
pub(crate) struct StatelessTracker<A: HalApi, Id: TypedId, T: Resource<Id>> {
    metadata: ResourceMetadata<A, Id, T>,

    _phantom: PhantomData<Id>,
}

impl<A: HalApi, Id: TypedId, T: Resource<Id>> StatelessTracker<A, Id, T> {
    pub fn new() -> Self {
        Self {
            metadata: ResourceMetadata::new(),

            _phantom: PhantomData,
        }
    }

    fn tracker_assert_in_bounds(&self, index: usize) {
        self.metadata.tracker_assert_in_bounds(index);
    }

    /// Sets the size of all the vectors inside the tracker.
    ///
    /// Must be called with the highest possible Resource ID of this type
    /// before all unsafe functions are called.
    pub fn set_size(&mut self, size: usize) {
        self.metadata.set_size(size);
    }

    /// Extend the vectors to let the given index be valid.
    fn allow_index(&mut self, index: usize) {
        if index >= self.metadata.size() {
            self.set_size(index + 1);
        }
    }

    /// Returns a list of all resources tracked.
    pub fn used_resources(&self) -> impl Iterator<Item = &Arc<T>> + '_ {
        self.metadata.owned_resources()
    }

    /// Inserts a single resource into the resource tracker.
    ///
    /// If the resource already exists in the tracker, it will be overwritten.
    ///
    /// If the ID is higher than the length of internal vectors,
    /// the vectors will be extended. A call to set_size is not needed.
    pub fn insert_single(&mut self, id: Valid<Id>, resource: Arc<T>) {
        let (index32, _epoch, _) = id.0.unzip();
        let index = index32 as usize;

        self.allow_index(index);

        self.tracker_assert_in_bounds(index);

        unsafe {
            self.metadata.insert(index, resource);
        }
    }

    /// Adds the given resource to the tracker.
    ///
    /// If the ID is higher than the length of internal vectors,
    /// the vectors will be extended. A call to set_size is not needed.
    pub fn add_single<'a>(&mut self, storage: &'a Storage<T, Id>, id: Id) -> Option<&'a T> {
        let resource = storage.get(id).ok()?;

        let (index32, _epoch, _) = id.unzip();
        let index = index32 as usize;

        self.allow_index(index);

        self.tracker_assert_in_bounds(index);

        unsafe {
            self.metadata.insert(index, resource.clone());
        }

        Some(resource)
    }

    /// Adds the given resources from the given tracker.
    ///
    /// If the ID is higher than the length of internal vectors,
    /// the vectors will be extended. A call to set_size is not needed.
    pub fn add_from_tracker(&mut self, other: &Self) {
        let incoming_size = other.metadata.size();
        if incoming_size > self.metadata.size() {
            self.set_size(incoming_size);
        }

        for index in other.metadata.owned_indices() {
            self.tracker_assert_in_bounds(index);
            other.tracker_assert_in_bounds(index);
            unsafe {
                let previously_owned = self.metadata.contains_unchecked(index);

                if !previously_owned {
                    let other_resource = other.metadata.get_resource_unchecked(index);
                    self.metadata.insert(index, other_resource.clone());
                }
            }
        }
    }

    /// Removes the given resource from the tracker iff we have the last reference to the
    /// resource and the epoch matches.
    ///
    /// Returns true if the resource was removed.
    ///
    /// If the ID is higher than the length of internal vectors,
    /// false will be returned.
    pub fn remove_abandoned(&mut self, id: Valid<Id>) -> bool {
        let index = id.0.unzip().0 as usize;

        if index > self.metadata.size() {
            return false;
        }

        self.tracker_assert_in_bounds(index);

        unsafe {
            if self.metadata.contains_unchecked(index) {
                let existing_ref_count = self.metadata.get_ref_count_unchecked(index);
                //3 ref count: Registry, Device Tracker and suspected resource itself
                if existing_ref_count <= 3 {
                    self.metadata.remove(index);
                    return true;
                } else {
                    log::info!("{:?} is still referenced from {}", self.metadata.get_resource_unchecked(index).label(), existing_ref_count);
                }
            }
        }

        false
    }
}
