from functools import lru_cache

@lru_cache
def cached_func(x):
    return x * 2

class MyClass:
    @property
    def my_prop(self):
        return 42

    def regular_method(self):
        return self.my_prop
