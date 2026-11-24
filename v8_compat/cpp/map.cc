#include "shim_types.h"

#include <cstddef>

namespace lexe_v8 {
namespace shim {

static_assert(offsetof(Map, instance_type) == 11);

static Map g_map_map(Map::MapMapTag::MapMap);
static Map g_object_map(InstanceType::Object);
static Map g_oddball_map(InstanceType::Oddball);
static Map g_string_map(InstanceType::String);
static Map g_heap_number_map(InstanceType::HeapNumber);

Map::Map(InstanceType instance_type)
    : meta_map(reinterpret_cast<uintptr_t>(&g_map_map)),
      header{0, 0, 0},
      instance_type(static_cast<uint8_t>(instance_type)),
      visitor_id(0) {}

Map::Map(MapMapTag)
    : meta_map(reinterpret_cast<uintptr_t>(this)),
      header{0, 0, 0},
      instance_type(static_cast<uint8_t>(InstanceType::Object)),
      visitor_id(0) {}

const Map& Map::map_map() { return g_map_map; }
const Map& Map::object_map() { return g_object_map; }
const Map& Map::oddball_map() { return g_oddball_map; }
const Map& Map::string_map() { return g_string_map; }
const Map& Map::heap_number_map() { return g_heap_number_map; }

}  // namespace shim
}  // namespace lexe_v8
