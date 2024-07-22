from sqlmodel import Field, SQLModel, Column, UniqueConstraint, Relationship
from typing import Any, TYPE_CHECKING
from uuid import uuid4, UUID
from geoalchemy2 import Geometry
from app.layers.links import LayerCountryLink

if TYPE_CHECKING:
    from app.layers.models import Layer


class Country(SQLModel, table=True):
    __table_args__ = (
        UniqueConstraint("id"),
        UniqueConstraint("name"),
    )
    iterator: int = Field(
        default=None,
        nullable=False,
        primary_key=True,
        index=True,
    )
    id: UUID = Field(
        default_factory=uuid4,
        index=True,
        nullable=False,
    )

    name: str = Field(
        index=True,
        nullable=False,
    )
    iso_a2: str = Field(
        index=True,
        nullable=False,
    )
    iso_a3: str = Field(
        index=True,
        nullable=False,
    )
    iso_n3: int = Field(
        index=True,
        nullable=False,
    )
    geom: Any = Field(
        default=None, sa_column=Column(Geometry("MULTIPOLYGON", srid=4326))
    )

    layers: list["Layer"] = Relationship(
        back_populates="countries",
        link_model=LayerCountryLink,
        sa_relationship_kwargs={"lazy": "selectin"},
    )

    layer_values: list[LayerCountryLink] = Relationship(
        back_populates="country",
        sa_relationship_kwargs={"lazy": "selectin"},
    )
