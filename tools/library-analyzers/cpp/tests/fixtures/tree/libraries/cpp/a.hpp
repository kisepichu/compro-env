#ifndef CE_FIXTURE_A_HPP
#define CE_FIXTURE_A_HPP
#include "b.hpp"
#include <ext.hpp>
#include "missing.hpp"
#define INC_D "d.hpp"
#include INC_D
#include "b.hpp"
#include "outside/x.hpp"
#include "日本語.hpp"
#if 0
#include "should_not_appear.hpp"
#endif
#endif  // CE_FIXTURE_A_HPP
