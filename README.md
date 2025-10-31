# yerba
A library providing a collection of allocators for different cases, some simple and specific, some more robust and general.

# Allocators

# TODO
- [x] Stack allocator
- [x] Contiguous list allocator
- [x] Fallback allocator
- [ ] InlineHeaderAllocator struct / InlineHeader trait
- [ ] Linked list header

The behavior of all list allocators with inline headers should be generalizable with all the implementation-specific code put in a set of different xInlineHeader structs
