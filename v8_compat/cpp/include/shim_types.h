#pragma once

#include <cstddef>
#include <cstdint>
#include <cstring>

namespace lexe_v8 {
namespace shim {

enum class InstanceType : uint16_t {
  String = 0x7f,
  Object = 0x422,
  HeapNumber = 0x82,
  Oddball = 0x83,
};

struct Map {
  uintptr_t meta_map;
  uint8_t header[3];
  uint8_t instance_type;
  uint8_t visitor_id;

  enum class MapMapTag { MapMap };

  Map(InstanceType instance_type);
  explicit Map(MapMapTag);

  static const Map& map_map();
  static const Map& object_map();
  static const Map& oddball_map();
  static const Map& string_map();
  static const Map& heap_number_map();
};

struct TaggedPointer {
  uintptr_t value;

  enum class Tag : uint8_t { Smi = 0, StrongPointer = 1, WeakPointer = 3 };

  TaggedPointer() : value(0) {}
  TaggedPointer(void* ptr, bool weak = false)
      : value(reinterpret_cast<uintptr_t>(ptr) |
              static_cast<uintptr_t>(weak ? Tag::WeakPointer : Tag::StrongPointer)) {}
  explicit TaggedPointer(int32_t smi)
      : value((static_cast<uintptr_t>(smi) << 32) | static_cast<uintptr_t>(Tag::Smi)) {}

  static TaggedPointer fromRaw(uintptr_t raw) {
    TaggedPointer t;
    t.value = raw;
    return t;
  }

  Tag tag() const { return static_cast<Tag>(value & 0b11); }
  bool isSmi() const { return tag() == Tag::Smi; }
  int32_t asSmi() const { return static_cast<int32_t>(value >> 32); }
  template <typename T>
  T* getPtr() const {
    return reinterpret_cast<T*>(value & ~static_cast<uintptr_t>(0b11));
  }
  uintptr_t* slot() { return &value; }
};

struct FakeHeapObject {
  uintptr_t map;
  uint64_t root_id;
};

struct ObjectLayout {
  TaggedPointer tagged_map;
  union {
    uint64_t root_id;
    double number;
  } contents;
  uint32_t function_id;
  uint8_t header_padding[12];
  void* embedder_slot_0;

  ObjectLayout() : tagged_map(0), contents{.root_id = 0}, function_id(0), embedder_slot_0(nullptr) {
    std::memset(header_padding, 0, sizeof(header_padding));
  }
  ObjectLayout(const Map* map, uint64_t root_id)
      : tagged_map(const_cast<Map*>(map)),
        contents{.root_id = root_id},
        function_id(0),
        embedder_slot_0(nullptr) {
    std::memset(header_padding, 0, sizeof(header_padding));
  }
  ObjectLayout(double number)
      : tagged_map(const_cast<Map*>(&Map::heap_number_map())),
        contents{.number = number},
        function_id(0),
        embedder_slot_0(nullptr) {
    std::memset(header_padding, 0, sizeof(header_padding));
  }
};

struct HandleSlot {
  TaggedPointer tagged_value;
  ObjectLayout object;
  bool owns_root;
};

}  // namespace shim

static_assert(offsetof(lexe_v8::shim::ObjectLayout, embedder_slot_0) == 32);

}  // namespace lexe_v8
