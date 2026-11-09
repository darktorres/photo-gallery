# The QML test host (main.cpp), built by tools/qmltest.py against the Qt kit (Quick and Qt Quick Test, the
# qmltestrunner's own libraries). Not part of the Cargo build.
cmake_minimum_required(VERSION 3.21)
project(qml_host LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 6.12 REQUIRED COMPONENTS Quick QuickTest)

add_executable(qml_host main.cpp)
target_link_libraries(qml_host PRIVATE Qt6::Quick Qt6::QuickTest)
