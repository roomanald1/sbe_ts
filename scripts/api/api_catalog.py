from common import catalogue

for service, endpoints in catalogue().items():
    print("\n" + service.upper())

    for name, route in endpoints.items():
        print(f"  {name:25} {route}")
