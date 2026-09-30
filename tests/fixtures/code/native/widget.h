#pragma once

#include <string>

class Widget {
public:
    Widget();
    std::string name() const;

private:
    int size_;
};
